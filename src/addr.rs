use memory_addr::{AddrRange, PhysAddr, VirtAddr, def_usize_addr, def_usize_addr_formatter};

/// Host virtual address.
pub type HostVirtAddr = VirtAddr;
/// Host physical address.
pub type HostPhysAddr = PhysAddr;

def_usize_addr! {
    /// Guest virtual address.
    pub type GuestVirtAddr;
    /// Guest physical address.
    pub type GuestPhysAddr;
}

/// Note: This is just a conversion in number and has no semantic meaning.
///
/// Why we need this conversion?
/// Because `GenericPTE` provided by `page_table_entry::x86_64` only accepts `PhysAddr` as the physical address type.
/// Introduce `GuestPhysAddr` concept into `GenericPTE` will bring a lot of complexity.
///
/// I just implement this ugly conversion to make things work.
impl From<PhysAddr> for GuestPhysAddr {
    fn from(addr: PhysAddr) -> Self {
        Self::from_usize(addr.into())
    }
}

def_usize_addr_formatter! {
    GuestVirtAddr = "GVA:{}";
    GuestPhysAddr = "GPA:{}";
}

/// Guest virtual address range.
pub type GuestVirtAddrRange = AddrRange<GuestVirtAddr>;
/// Guest physical address range.
pub type GuestPhysAddrRange = AddrRange<GuestPhysAddr>;

#[cfg(any(target_arch = "riscv32", target_arch = "riscv64"))]
impl page_table_multiarch::riscv::SvVirtAddr for GuestPhysAddr {
    fn flush_tlb(_vaddr: Option<Self>) {
        todo!()
    }
}
