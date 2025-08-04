//! Unified guest memory access interface
//!
//! This module provides a safe and consistent way to access guest memory
//! from VirtIO device implementations, handling address translation and
//! memory safety concerns.
use crate::GuestPhysAddr;
use axerrno::{AxError, AxResult};
use memory_addr::PhysAddr;

/// Trait for address translation
pub trait AddressTranslator {
    /// Translate a guest physical address to host physical address
    fn translate_guest_to_host(&self, guest_addr: GuestPhysAddr) -> Option<PhysAddr>;
}

/// Guest memory access with injected translator
#[derive(Debug, Clone)]
pub struct GuestMemoryAccessor<T> {
    translator: T,
}

impl<T: AddressTranslator> GuestMemoryAccessor<T> {
    /// Create a new guest memory accessor
    pub fn new(translator: T) -> Self {
        Self { translator }
    }
}

impl<T: AddressTranslator> GuestMemoryAccessor<T> {
    /// Read a value of type V from guest memory
    pub fn read_obj<V: Copy>(&self, guest_addr: GuestPhysAddr) -> AxResult<V> {
        let host_addr = self
            .translator
            .translate_guest_to_host(guest_addr)
            .ok_or(AxError::InvalidInput)?;

        unsafe {
            let ptr = host_addr.as_usize() as *const V;
            Ok(core::ptr::read_volatile(ptr))
        }
    }

    /// Write a value of type V to guest memory
    pub fn write_obj<V: Copy>(&self, guest_addr: GuestPhysAddr, val: V) -> AxResult<()> {
        let host_addr = self
            .translator
            .translate_guest_to_host(guest_addr)
            .ok_or(AxError::InvalidInput)?;

        unsafe {
            let ptr = host_addr.as_usize() as *mut V;
            core::ptr::write_volatile(ptr, val);
        }
        Ok(())
    }

    /// Read a buffer from guest memory
    pub fn read_buffer(&self, guest_addr: GuestPhysAddr, buffer: &mut [u8]) -> AxResult<()> {
        let host_addr = self
            .translator
            .translate_guest_to_host(guest_addr)
            .ok_or(AxError::InvalidInput)?;

        unsafe {
            let src_ptr = host_addr.as_usize() as *const u8;
            core::ptr::copy_nonoverlapping(src_ptr, buffer.as_mut_ptr(), buffer.len());
        }
        Ok(())
    }

    /// Write a buffer to guest memory
    pub fn write_buffer(&self, guest_addr: GuestPhysAddr, buffer: &[u8]) -> AxResult<()> {
        let host_addr = self
            .translator
            .translate_guest_to_host(guest_addr)
            .ok_or(AxError::InvalidInput)?;

        unsafe {
            let dst_ptr = host_addr.as_usize() as *mut u8;
            core::ptr::copy_nonoverlapping(buffer.as_ptr(), dst_ptr, buffer.len());
        }
        Ok(())
    }

    /// Read a volatile value from guest memory (for device registers)
    pub fn read_volatile<V: Copy>(&self, guest_addr: GuestPhysAddr) -> AxResult<V> {
        self.read_obj(guest_addr)
    }

    /// Write a volatile value to guest memory (for device registers)
    pub fn write_volatile<V: Copy>(&self, guest_addr: GuestPhysAddr, val: V) -> AxResult<()> {
        self.write_obj(guest_addr, val)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::{BASE_PADDR, mock_hal_test};
    use alloc::vec;
    use alloc::vec::Vec;
    use axerrno::AxError;
    use memory_addr::PhysAddr;

    /// Mock address translator for testing
    #[derive(Clone)]
    struct MockTranslator {
        /// Whether translation should fail
        fail_translation: bool,
    }

    impl MockTranslator {
        fn new() -> Self {
            Self {
                fail_translation: false,
            }
        }

        fn new_failing() -> Self {
            Self {
                fail_translation: true,
            }
        }
    }

    impl AddressTranslator for MockTranslator {
        fn translate_guest_to_host(&self, guest_addr: GuestPhysAddr) -> Option<PhysAddr> {
            if self.fail_translation {
                return None;
            }

            // Simple 1:1 mapping for testing, offset by BASE_PADDR
            let guest_offset = guest_addr.as_usize();
            if guest_offset < 0x10000 {
                // Within our test memory range
                // Convert to physical address first, then to virtual address
                let host_paddr = PhysAddr::from_usize(BASE_PADDR + guest_offset);
                let host_vaddr = crate::test_utils::MockHal::mock_phys_to_virt(host_paddr);
                Some(PhysAddr::from_usize(host_vaddr.as_usize()))
            } else {
                None
            }
        }
    }

    #[test]
    fn test_accessor_creation() {
        mock_hal_test(|| {
            let translator = MockTranslator::new();
            let accessor = GuestMemoryAccessor::new(translator);

            // Test that accessor can be created and cloned
            let _cloned_accessor = accessor.clone();
        });
    }

    #[test]
    fn test_read_write_obj() {
        mock_hal_test(|| {
            let translator = MockTranslator::new();
            let accessor = GuestMemoryAccessor::new(translator);

            let guest_addr = GuestPhysAddr::from_usize(0x100);
            let test_value: u32 = 0x12345678;

            // Write a value
            accessor.write_obj(guest_addr, test_value).unwrap();

            // Read it back
            let read_value: u32 = accessor.read_obj(guest_addr).unwrap();
            assert_eq!(read_value, test_value);
        });
    }

    #[test]
    fn test_read_write_different_types() {
        mock_hal_test(|| {
            let translator = MockTranslator::new();
            let accessor = GuestMemoryAccessor::new(translator);

            // Test u8
            let guest_addr_u8 = GuestPhysAddr::from_usize(0x200);
            let test_u8: u8 = 0xAB;
            accessor.write_obj(guest_addr_u8, test_u8).unwrap();
            let read_u8: u8 = accessor.read_obj(guest_addr_u8).unwrap();
            assert_eq!(read_u8, test_u8);

            // Test u16
            let guest_addr_u16 = GuestPhysAddr::from_usize(0x300);
            let test_u16: u16 = 0x1234;
            accessor.write_obj(guest_addr_u16, test_u16).unwrap();
            let read_u16: u16 = accessor.read_obj(guest_addr_u16).unwrap();
            assert_eq!(read_u16, test_u16);

            // Test u64
            let guest_addr_u64 = GuestPhysAddr::from_usize(0x400);
            let test_u64: u64 = 0x123456789ABCDEF0;
            accessor.write_obj(guest_addr_u64, test_u64).unwrap();
            let read_u64: u64 = accessor.read_obj(guest_addr_u64).unwrap();
            assert_eq!(read_u64, test_u64);
        });
    }

    #[test]
    fn test_read_write_buffer() {
        mock_hal_test(|| {
            let translator = MockTranslator::new();
            let accessor = GuestMemoryAccessor::new(translator);

            let guest_addr = GuestPhysAddr::from_usize(0x500);
            let test_data = b"Hello, World! This is a test buffer.";

            // Write buffer
            accessor.write_buffer(guest_addr, test_data).unwrap();

            // Read buffer back
            let mut read_buffer = vec![0u8; test_data.len()];
            accessor.read_buffer(guest_addr, &mut read_buffer).unwrap();

            assert_eq!(read_buffer.as_slice(), test_data);
        });
    }

    #[test]
    fn test_volatile_operations() {
        mock_hal_test(|| {
            let translator = MockTranslator::new();
            let accessor = GuestMemoryAccessor::new(translator);

            let guest_addr = GuestPhysAddr::from_usize(0x600);
            let test_value: u32 = 0xDEADBEEF;

            // Write volatile
            accessor.write_volatile(guest_addr, test_value).unwrap();

            // Read volatile
            let read_value: u32 = accessor.read_volatile(guest_addr).unwrap();
            assert_eq!(read_value, test_value);
        });
    }

    #[test]
    fn test_translation_failure() {
        mock_hal_test(|| {
            let translator = MockTranslator::new_failing();
            let accessor = GuestMemoryAccessor::new(translator);

            let guest_addr = GuestPhysAddr::from_usize(0x700);
            let test_value: u32 = 0x12345678;

            // All operations should fail with InvalidInput when translation fails
            assert!(matches!(
                accessor.write_obj(guest_addr, test_value),
                Err(AxError::InvalidInput)
            ));

            assert!(matches!(
                accessor.read_obj::<u32>(guest_addr),
                Err(AxError::InvalidInput)
            ));

            let mut buffer = [0u8; 10];
            assert!(matches!(
                accessor.read_buffer(guest_addr, &mut buffer),
                Err(AxError::InvalidInput)
            ));

            let test_buffer = b"test";
            assert!(matches!(
                accessor.write_buffer(guest_addr, test_buffer),
                Err(AxError::InvalidInput)
            ));

            assert!(matches!(
                accessor.read_volatile::<u32>(guest_addr),
                Err(AxError::InvalidInput)
            ));

            assert!(matches!(
                accessor.write_volatile(guest_addr, test_value),
                Err(AxError::InvalidInput)
            ));
        });
    }

    #[test]
    fn test_out_of_bounds_translation() {
        mock_hal_test(|| {
            let translator = MockTranslator::new();
            let accessor = GuestMemoryAccessor::new(translator);

            // Try to access an address that's out of our mock memory range
            let guest_addr = GuestPhysAddr::from_usize(0x20000); // Beyond our 64KB test range
            let test_value: u32 = 0x12345678;

            // Should fail because translation returns None for out-of-bounds addresses
            assert!(matches!(
                accessor.write_obj(guest_addr, test_value),
                Err(AxError::InvalidInput)
            ));

            assert!(matches!(
                accessor.read_obj::<u32>(guest_addr),
                Err(AxError::InvalidInput)
            ));
        });
    }

    #[test]
    fn test_zero_length_buffer() {
        mock_hal_test(|| {
            let translator = MockTranslator::new();
            let accessor = GuestMemoryAccessor::new(translator);

            let guest_addr = GuestPhysAddr::from_usize(0x800);

            // Test with zero-length buffers
            let empty_write_buffer: &[u8] = &[];
            accessor
                .write_buffer(guest_addr, empty_write_buffer)
                .unwrap();

            let empty_read_buffer: &mut [u8] = &mut [];
            accessor.read_buffer(guest_addr, empty_read_buffer).unwrap();
        });
    }

    #[test]
    fn test_large_buffer() {
        mock_hal_test(|| {
            let translator = MockTranslator::new();
            let accessor = GuestMemoryAccessor::new(translator);

            let guest_addr = GuestPhysAddr::from_usize(0x1000);

            // Create a large buffer (but within our test memory limits)
            let large_data: Vec<u8> = (0..1000).map(|i| (i % 256) as u8).collect();

            // Write large buffer
            accessor.write_buffer(guest_addr, &large_data).unwrap();

            // Read it back
            let mut read_buffer = vec![0u8; large_data.len()];
            accessor.read_buffer(guest_addr, &mut read_buffer).unwrap();

            assert_eq!(read_buffer, large_data);
        });
    }
}
