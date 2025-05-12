use core::{convert::TryFrom, fmt};

use bit_field::BitField;
use bitflags::bitflags;
use memory_addr::PAGE_SIZE_4K;
use page_table_entry::{GenericPTE, MappingFlags};
use page_table_multiarch::{PageTable64, PagingMetaData};

use crate::{GuestPhysAddr, HostPhysAddr};

bitflags::bitflags! {
    /// EPT entry flags. (SDM Vol. 3C, Section 28.3.2)
    struct EPTFlags: u64 {
        /// Read access.
        const READ =                1 << 0;
        /// Write access.
        const WRITE =               1 << 1;
        /// Execute access.
        const EXECUTE =             1 << 2;
        /// EPT memory type. Only for terminate pages.
        const MEM_TYPE_MASK =       0b111 << 3;
        /// Ignore PAT memory type. Only for terminate pages.
        const IGNORE_PAT =          1 << 6;
        /// Specifies that the entry maps a huge frame instead of a page table.
        /// Only allowed in P2 or P3 tables.
        const HUGE_PAGE =           1 << 7;
        /// If bit 6 of EPTP is 1, accessed flag for EPT.
        const ACCESSED =            1 << 8;
        /// If bit 6 of EPTP is 1, dirty flag for EPT.
        const DIRTY =               1 << 9;
        /// Execute access for user-mode linear addresses.
        const EXECUTE_FOR_USER =    1 << 10;
    }
}

numeric_enum_macro::numeric_enum! {
    #[repr(u8)]
    #[derive(Debug, PartialEq, Clone, Copy)]
    /// EPT memory typing. (SDM Vol. 3C, Section 28.3.7)
    enum EPTMemType {
        Uncached = 0,
        WriteCombining = 1,
        WriteThrough = 4,
        WriteProtected = 5,
        WriteBack = 6,
    }
}

impl EPTFlags {
    fn set_mem_type(&mut self, mem_type: EPTMemType) {
        let mut bits = self.bits();
        bits.set_bits(3..6, mem_type as u64);
        *self = Self::from_bits_truncate(bits)
    }
    fn mem_type(&self) -> Result<EPTMemType, u8> {
        EPTMemType::try_from(self.bits().get_bits(3..6) as u8)
    }
}

impl From<MappingFlags> for EPTFlags {
    fn from(f: MappingFlags) -> Self {
        if f.is_empty() {
            return Self::empty();
        }
        let mut ret = Self::empty();
        if f.contains(MappingFlags::READ) {
            ret |= Self::READ;
        }
        if f.contains(MappingFlags::WRITE) {
            ret |= Self::WRITE;
        }
        if f.contains(MappingFlags::EXECUTE) {
            ret |= Self::EXECUTE;
        }
        if !f.contains(MappingFlags::DEVICE) {
            ret.set_mem_type(EPTMemType::WriteBack);
        }
        ret
    }
}

impl From<EPTFlags> for MappingFlags {
    fn from(f: EPTFlags) -> Self {
        let mut ret = MappingFlags::empty();
        if f.contains(EPTFlags::READ) {
            ret |= Self::READ;
        }
        if f.contains(EPTFlags::WRITE) {
            ret |= Self::WRITE;
        }
        if f.contains(EPTFlags::EXECUTE) {
            ret |= Self::EXECUTE;
        }
        if let Ok(EPTMemType::Uncached) = f.mem_type() {
            ret |= Self::DEVICE;
        }
        ret
    }
}

/// An x86_64 VMX extented page table entry.
/// Note: The [EPTEntry] can be moved to the independent crate `page_table_entry`.
#[derive(Clone, Copy)]
#[repr(transparent)]
pub struct EPTEntry(u64);

impl EPTEntry {
    const PHYS_ADDR_MASK: u64 = 0x000f_ffff_ffff_f000; // bits 12..52
}

impl GenericPTE for EPTEntry {
    fn new_page(paddr: HostPhysAddr, flags: MappingFlags, is_huge: bool) -> Self {
        let mut flags = EPTFlags::from(flags);
        if is_huge {
            flags |= EPTFlags::HUGE_PAGE;
        }
        Self(flags.bits() | (paddr.as_usize() as u64 & Self::PHYS_ADDR_MASK))
    }
    fn new_table(paddr: HostPhysAddr) -> Self {
        let flags = EPTFlags::READ | EPTFlags::WRITE | EPTFlags::EXECUTE;
        Self(flags.bits() | (paddr.as_usize() as u64 & Self::PHYS_ADDR_MASK))
    }
    fn paddr(&self) -> HostPhysAddr {
        HostPhysAddr::from((self.0 & Self::PHYS_ADDR_MASK) as usize)
    }
    fn flags(&self) -> MappingFlags {
        EPTFlags::from_bits_truncate(self.0).into()
    }
    fn set_paddr(&mut self, paddr: HostPhysAddr) {
        self.0 = (self.0 & !Self::PHYS_ADDR_MASK) | (paddr.as_usize() as u64 & Self::PHYS_ADDR_MASK)
    }

    fn set_flags(&mut self, flags: MappingFlags, is_huge: bool) {
        let mut flags = EPTFlags::from(flags);
        if is_huge {
            flags |= EPTFlags::HUGE_PAGE;
        }
        self.0 = (self.0 & Self::PHYS_ADDR_MASK) | flags.bits()
    }
    fn is_unused(&self) -> bool {
        self.0 == 0
    }
    fn is_present(&self) -> bool {
        self.0 & 0x7 != 0 // RWX != 0
    }
    fn is_huge(&self) -> bool {
        EPTFlags::from_bits_truncate(self.0).contains(EPTFlags::HUGE_PAGE)
    }
    fn clear(&mut self) {
        self.0 = 0
    }

    fn bits(self) -> usize {
        self.0 as usize
    }
}

impl fmt::Debug for EPTEntry {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("EPTEntry")
            .field("raw", &self.0)
            .field("hpaddr", &self.paddr())
            .field("flags", &self.flags())
            .field("mem_type", &EPTFlags::from_bits_truncate(self.0).mem_type())
            .finish()
    }
}

/// Metadata of VMX extended page tables.
pub struct ExtendedPageTableMetadata;

impl PagingMetaData for ExtendedPageTableMetadata {
    const LEVELS: usize = 4;
    const PA_MAX_BITS: usize = 52;
    const VA_MAX_BITS: usize = 48;

    type VirtAddr = GuestPhysAddr;

    fn flush_tlb(_vaddr: Option<GuestPhysAddr>) {
        // Todo: more efficient way to flush TLB.
        unsafe {
            invept(InvEptType::Global);
        }
    }
}

/// The VMX extended page table. (SDM Vol. 3C, Section 29.3)
pub type ExtendedPageTable<H> = PageTable64<ExtendedPageTableMetadata, EPTEntry, H>;

/// INVEPT type. (SDM Vol. 3C, Section 30.3)
#[repr(u64)]
#[derive(Debug)]
#[allow(dead_code)]
pub enum InvEptType {
    /// The logical processor invalidates all mappings associated with bits
    /// 51:12 of the EPT pointer (EPTP) specified in the INVEPT descriptor.
    /// It may invalidate other mappings as well.
    SingleContext = 1,
    /// The logical processor invalidates mappings associated with all EPTPs.
    Global = 2,
}

/// Invalidate Translations Derived from EPT. (SDM Vol. 3C, Section 30.3)
///
/// Invalidates mappings in the translation lookaside buffers (TLBs) and
/// paging-structure caches that were derived from extended page tables (EPT).
/// (See Chapter 28, “VMX Support for Address Translation”.) Invalidation is
/// based on the INVEPT type specified in the register operand and the INVEPT
/// descriptor specified in the memory operand.
pub unsafe fn invept(inv_type: InvEptType) {
    let invept_desc = [0, 0];
    unsafe {
        core::arch::asm!("invept {0}, [{1}]", in(reg) inv_type as u64, in(reg) &invept_desc);
    }
}

bitflags! {
    /// Extended-Page-Table Pointer. (SDM Vol. 3C, Section 24.6.11)
    #[derive(Clone, Copy)]
    pub struct EPTPointer: u64 {
        /// EPT paging-structure memory type: Uncacheable (UC).
        #[allow(clippy::identity_op)]
        const MEM_TYPE_UC = 0 << 0;
        /// EPT paging-structure memory type: Write-back (WB).
        #[allow(clippy::identity_op)]
        const MEM_TYPE_WB = 6 << 0;
        /// EPT page-walk length 1.
        const WALK_LENGTH_1 = 0 << 3;
        /// EPT page-walk length 2.
        const WALK_LENGTH_2 = 1 << 3;
        /// EPT page-walk length 3.
        const WALK_LENGTH_3 = 2 << 3;
        /// EPT page-walk length 4.
        const WALK_LENGTH_4 = 3 << 3;
        /// EPT page-walk length 5
        const WALK_LENGTH_5 = 4 << 3;
        /// Setting this control to 1 enables accessed and dirty flags for EPT.
        const ENABLE_ACCESSED_DIRTY = 1 << 6;
    }
}

impl EPTPointer {
    pub fn from_table_phys(pml4_paddr: HostPhysAddr) -> Self {
        let aligned_addr = pml4_paddr.as_usize() & !(PAGE_SIZE_4K - 1);
        let flags = Self::from_bits_retain(aligned_addr as u64);
        flags | Self::MEM_TYPE_WB | Self::WALK_LENGTH_4 | Self::ENABLE_ACCESSED_DIRTY
    }

    pub fn into_ept_root(&self) -> HostPhysAddr {
        const PHYS_ADDR_MASK: usize = 0x000f_ffff_ffff_f000; // bits 12..52
        HostPhysAddr::from(self.bits() as usize & PHYS_ADDR_MASK)
    }
}

impl fmt::Debug for EPTPointer {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("EPTPointer")
            .field("raw", &self.0)
            .field("ept_root", &self.into_ept_root())
            .finish()
    }
}
