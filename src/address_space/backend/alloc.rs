use memory_addr::{
    MemoryAddr, PAGE_SIZE_1G, PAGE_SIZE_2M, PAGE_SIZE_4K, PageIter1G, PageIter2M, PageIter4K,
    PhysAddr,
};
use page_table_multiarch::{
    GenericPTE, MappingFlags, PageSize, PageTable64, PagingHandler, PagingMetaData,
};

use super::Backend;

impl<M: PagingMetaData, PTE: GenericPTE, H: PagingHandler> Backend<M, PTE, H> {
    /// Creates a new allocation mapping backend.
    pub const fn new_alloc(populate: bool) -> Self {
        Self::Alloc {
            populate,
            _phantom: core::marker::PhantomData,
        }
    }

    pub(crate) fn map_alloc(
        &self,
        start: M::VirtAddr,
        size: usize,
        flags: MappingFlags,
        pt: &mut PageTable64<M, PTE, H>,
        populate: bool,
    ) -> bool {
        debug!(
            "map_alloc: [{:#x}, {:#x}) {:?} (populate={})",
            start,
            start.add(size),
            flags,
            populate
        );
        if populate {
            // allocate all possible physical frames for populated mapping.

            let mut start_addr = start;
            let end_addr = start_addr.add(size);
            // First try to allocate 1GB pages if the start address is aligned and
            // the size is large enough.
            if start_addr.is_aligned(PAGE_SIZE_1G) && size >= PAGE_SIZE_1G {
                for addr in PageIter1G::new(start_addr, end_addr.align_down(PAGE_SIZE_1G)).unwrap()
                {
                    if H::alloc_frames(PAGE_SIZE_1G / PAGE_SIZE_4K, PAGE_SIZE_1G)
                        .and_then(|frame| pt.map(addr, frame, PageSize::Size1G, flags).ok())
                        .is_none()
                    {
                        return false;
                    }
                }
                start_addr = end_addr;
            }

            // Then try to allocate 2MB pages if the start address is aligned and
            // the size is large enough.
            if start_addr.is_aligned(PAGE_SIZE_2M) && size >= PAGE_SIZE_2M {
                for addr in PageIter2M::new(start_addr, end_addr.align_down(PAGE_SIZE_2M)).unwrap()
                {
                    if H::alloc_frames(PAGE_SIZE_2M / PAGE_SIZE_4K, PAGE_SIZE_2M)
                        .and_then(|frame| pt.map(addr, frame, PageSize::Size2M, flags).ok())
                        .is_none()
                    {
                        return false;
                    }
                }
                start_addr = end_addr;
            }

            // Then try to allocate 4K pages.
            for addr in PageIter4K::new(start_addr, end_addr).unwrap() {
                if H::alloc_frame()
                    .and_then(|frame| pt.map(addr, frame, PageSize::Size4K, flags).ok())
                    .is_none()
                {
                    return false;
                }
            }
            true
        } else {
            // Map to a empty entry for on-demand mapping.
            pt.map_region(
                start,
                |_va| PhysAddr::from(0),
                size,
                MappingFlags::empty(),
                false,
                false,
            )
            .is_ok()
        }
    }

    pub(crate) fn unmap_alloc(
        &self,
        start: M::VirtAddr,
        size: usize,
        pt: &mut PageTable64<M, PTE, H>,
        _populate: bool,
    ) -> bool {
        debug!("unmap_alloc: [{:#x}, {:#x})", start, start.add(size));

        let mut addr = start;
        while addr < start.add(size) {
            if let Ok((frame, _flags, page_size)) = pt.query(addr) {
                // Deallocate the physical frame if there is a mapping in the
                // page table.
                match page_size {
                    PageSize::Size1G => {
                        if !addr.is_aligned(PAGE_SIZE_1G) {
                            return false;
                        }
                        H::dealloc_frames(frame, PAGE_SIZE_1G / PAGE_SIZE_4K);
                    }
                    PageSize::Size2M => {
                        if !addr.is_aligned(PAGE_SIZE_2M) {
                            return false;
                        }
                        H::dealloc_frames(frame, PAGE_SIZE_2M / PAGE_SIZE_4K);
                    }
                    PageSize::Size4K => {
                        if !addr.is_aligned(PAGE_SIZE_4K) {
                            return false;
                        }
                        H::dealloc_frame(frame);
                    }
                }
                addr = addr.add(page_size as usize);
            } else {
                // It's fine if the page is not mapped.
            }
        }

        for addr in PageIter4K::new(start, start.add(size)).unwrap() {
            if let Ok((frame, page_size, _)) = pt.unmap(addr) {
                // Deallocate the physical frame if there is a mapping in the
                // page table.
                if page_size.is_huge() {
                    return false;
                }
                H::dealloc_frame(frame);
            } else {
                // It's fine if the page is not mapped.
            }
        }
        true
    }

    pub(crate) fn handle_page_fault_alloc(
        &self,
        vaddr: M::VirtAddr,
        orig_flags: MappingFlags,
        pt: &mut PageTable64<M, PTE, H>,
        populate: bool,
    ) -> bool {
        if populate {
            false // Populated mappings should not trigger page faults.
        } else {
            // Allocate a physical frame lazily and map it to the fault address.
            // `vaddr` does not need to be aligned. It will be automatically
            // aligned during `pt.remap` regardless of the page size.
            H::alloc_frame()
                .and_then(|frame| pt.remap(vaddr, frame, orig_flags).ok())
                .is_some()
        }
    }
}
