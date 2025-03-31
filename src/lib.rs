//! [ArceOS-Hypervisor](https://github.com/arceos-hypervisor/) guest VM address space management module.

#![no_std]
#![feature(const_trait_impl)]

#[macro_use]
extern crate log;
extern crate alloc;

mod addr;
mod address_space;
/// Todo: this has to be combined with page_table_multiarch with `nested_page_table` feature,
/// or separated into a new crate maybe named as `nested_page_table_multiarch`.
pub mod npt;

pub use addr::*;
pub use address_space::*;

use axerrno::AxError;
use memory_set::MappingError;

/// Information about nested page faults.
#[derive(Debug)]
pub struct NestedPageFaultInfo {
    /// Access type that caused the nested page fault.
    pub access_flags: MappingFlags,
    /// Guest physical address that caused the nested page fault.
    pub fault_guest_paddr: GuestPhysAddr,
}

fn mapping_err_to_ax_err(err: MappingError) -> AxError {
    warn!("Mapping error: {:?}", err);
    match err {
        MappingError::InvalidParam => AxError::InvalidInput,
        MappingError::AlreadyExists => AxError::AlreadyExists,
        MappingError::BadState => AxError::BadState,
    }
}

pub trait EPTTranslator {
    /// Converts a guest physical address to a host physical address
    /// through Nested Page Table (NPT) translation.
    ///
    /// # Parameters
    ///
    /// * `gpa` - The guest physical address to convert.
    ///
    /// # Returns
    ///
    /// * `HostPhysAddr` - The corresponding host physical address.
    fn guest_phys_to_host_phys(gpa: GuestPhysAddr) -> Option<HostPhysAddr>;
}
