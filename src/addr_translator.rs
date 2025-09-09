//! Unified guest memory access interface
//!
//! This module provides a safe and consistent way to access guest memory
//! from VirtIO device implementations, handling address translation and
//! memory safety concerns.
use crate::GuestPhysAddr;
use axerrno::{AxError, AxResult};
use memory_addr::{PAGE_SIZE_4K, PhysAddr};

/// Trait for address translation
pub trait AddressTranslator {
    /// Translate a guest physical address to host physical address
    fn translate_guest_to_host(&self, guest_addr: GuestPhysAddr) -> Option<PhysAddr>;

    /// Get the page size for a given guest address
    ///
    /// This allows implementations to provide actual page size information
    /// based on their memory management configuration.
    /// Default implementation returns 4KB page size.
    fn get_page_size(&self, _guest_addr: GuestPhysAddr) -> usize {
        PAGE_SIZE_4K
    }

    /// Check if an access crosses page boundary
    ///
    /// This function checks whether accessing `size` bytes starting from `guest_addr`
    /// would cross the boundary of a page. Uses the page size from get_page_size().
    /// Default implementation provides standard page boundary checking logic.
    fn crosses_page_boundary(&self, guest_addr: GuestPhysAddr, size: usize) -> bool {
        if size == 0 {
            return false;
        }

        let page_size = self.get_page_size(guest_addr);
        let start_page = guest_addr.as_usize() & !(page_size - 1);
        let end_addr = guest_addr.as_usize() + size - 1;
        let end_page = end_addr & !(page_size - 1);

        start_page != end_page
    }

    /// Read a value of type V from guest memory
    fn read_obj<V: Copy>(&self, guest_addr: GuestPhysAddr) -> AxResult<V> {
        let host_addr = self
            .translate_guest_to_host(guest_addr)
            .ok_or(AxError::InvalidInput)?;

        unsafe {
            let ptr = host_addr.as_usize() as *const V;
            Ok(core::ptr::read_volatile(ptr))
        }
    }

    /// Write a value of type V to guest memory
    fn write_obj<V: Copy>(&self, guest_addr: GuestPhysAddr, val: V) -> AxResult<()> {
        let host_addr = self
            .translate_guest_to_host(guest_addr)
            .ok_or(AxError::InvalidInput)?;

        unsafe {
            let ptr = host_addr.as_usize() as *mut V;
            core::ptr::write_volatile(ptr, val);
        }
        Ok(())
    }

    /// Read a buffer from guest memory
    fn read_buffer(&self, guest_addr: GuestPhysAddr, buffer: &mut [u8]) -> AxResult<()> {
        if buffer.is_empty() {
            return Ok(());
        }

        // Check if the access crosses page boundary using the trait method
        if !self.crosses_page_boundary(guest_addr, buffer.len()) {
            // Simple case: single page access
            let host_addr = self
                .translate_guest_to_host(guest_addr)
                .ok_or(AxError::InvalidInput)?;

            unsafe {
                let src_ptr = host_addr.as_usize() as *const u8;
                core::ptr::copy_nonoverlapping(src_ptr, buffer.as_mut_ptr(), buffer.len());
            }
            return Ok(());
        }

        // Complex case: cross-page access, handle page by page
        let mut current_guest_addr = guest_addr;
        let mut remaining_buffer = buffer;

        while !remaining_buffer.is_empty() {
            // Get page size for current address
            let page_size = self.get_page_size(current_guest_addr);

            // Calculate how much we can read from current page
            let page_offset = current_guest_addr.as_usize() & (page_size - 1);
            let bytes_in_current_page = page_size - page_offset;
            let bytes_to_read = remaining_buffer.len().min(bytes_in_current_page);

            // Translate current page address
            let host_addr = self
                .translate_guest_to_host(current_guest_addr)
                .ok_or(AxError::InvalidInput)?;

            // Read from current page
            unsafe {
                let src_ptr = host_addr.as_usize() as *const u8;
                core::ptr::copy_nonoverlapping(
                    src_ptr,
                    remaining_buffer.as_mut_ptr(),
                    bytes_to_read,
                );
            }

            // Move to next page
            current_guest_addr =
                GuestPhysAddr::from_usize(current_guest_addr.as_usize() + bytes_to_read);
            remaining_buffer = &mut remaining_buffer[bytes_to_read..];
        }

        Ok(())
    }

    /// Write a buffer to guest memory
    fn write_buffer(&self, guest_addr: GuestPhysAddr, buffer: &[u8]) -> AxResult<()> {
        if buffer.is_empty() {
            return Ok(());
        }

        // Check if the access crosses page boundary using the trait method
        if !self.crosses_page_boundary(guest_addr, buffer.len()) {
            // Simple case: single page access
            let host_addr = self
                .translate_guest_to_host(guest_addr)
                .ok_or(AxError::InvalidInput)?;

            unsafe {
                let dst_ptr = host_addr.as_usize() as *mut u8;
                core::ptr::copy_nonoverlapping(buffer.as_ptr(), dst_ptr, buffer.len());
            }
            return Ok(());
        }

        // Complex case: cross-page access, handle page by page
        let mut current_guest_addr = guest_addr;
        let mut remaining_buffer = buffer;

        while !remaining_buffer.is_empty() {
            // Get page size for current address
            let page_size = self.get_page_size(current_guest_addr);

            // Calculate how much we can write to current page
            let page_offset = current_guest_addr.as_usize() & (page_size - 1);
            let bytes_in_current_page = page_size - page_offset;
            let bytes_to_write = remaining_buffer.len().min(bytes_in_current_page);

            // Translate current page address
            let host_addr = self
                .translate_guest_to_host(current_guest_addr)
                .ok_or(AxError::InvalidInput)?;

            // Write to current page
            unsafe {
                let dst_ptr = host_addr.as_usize() as *mut u8;
                core::ptr::copy_nonoverlapping(remaining_buffer.as_ptr(), dst_ptr, bytes_to_write);
            }

            // Move to next page
            current_guest_addr =
                GuestPhysAddr::from_usize(current_guest_addr.as_usize() + bytes_to_write);
            remaining_buffer = &remaining_buffer[bytes_to_write..];
        }

        Ok(())
    }

    /// Read a volatile value from guest memory (for device registers)
    fn read_volatile<V: Copy>(&self, guest_addr: GuestPhysAddr) -> AxResult<V> {
        self.read_obj(guest_addr)
    }

    /// Write a volatile value to guest memory (for device registers)
    fn write_volatile<V: Copy>(&self, guest_addr: GuestPhysAddr, val: V) -> AxResult<()> {
        self.write_obj(guest_addr, val)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::{BASE_PADDR, mock_hal_test};
    use axin::axin;
    use memory_addr::PhysAddr;

    /// Mock implementation of AddressTranslator for testing
    struct MockTranslator {
        base_addr: PhysAddr,
        memory_size: usize,
    }

    impl MockTranslator {
        pub fn new(base_addr: PhysAddr, memory_size: usize) -> Self {
            Self {
                base_addr,
                memory_size,
            }
        }
    }

    impl AddressTranslator for MockTranslator {
        fn translate_guest_to_host(&self, guest_addr: GuestPhysAddr) -> Option<PhysAddr> {
            // Simple mapping: guest address directly maps to mock memory region
            let offset = guest_addr.as_usize();
            if offset < self.memory_size {
                // Convert physical address to virtual address for actual memory access
                let phys_addr =
                    PhysAddr::from_usize(BASE_PADDR + self.base_addr.as_usize() + offset);
                let virt_addr = crate::test_utils::MockHal::mock_phys_to_virt(phys_addr);
                Some(PhysAddr::from_usize(virt_addr.as_usize()))
            } else {
                None
            }
        }
    }

    #[test]
    #[axin(decorator(mock_hal_test))]
    fn test_basic_read_write_operations() {
        let translator =
            MockTranslator::new(PhysAddr::from_usize(0), crate::test_utils::MEMORY_LEN);

        // Test u32 read/write operations
        let test_addr = GuestPhysAddr::from_usize(0x100);
        let test_value: u32 = 0x12345678;

        // Write a u32 value
        translator
            .write_obj(test_addr, test_value)
            .expect("Failed to write u32 value");

        // Read back the u32 value
        let read_value: u32 = translator
            .read_obj(test_addr)
            .expect("Failed to read u32 value");

        assert_eq!(
            read_value, test_value,
            "Read value should match written value"
        );

        // Test buffer read/write operations
        let buffer_addr = GuestPhysAddr::from_usize(0x200);
        let test_buffer = [0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88];

        // Write buffer
        translator
            .write_buffer(buffer_addr, &test_buffer)
            .expect("Failed to write buffer");

        // Read buffer back
        let mut read_buffer = [0u8; 8];
        translator
            .read_buffer(buffer_addr, &mut read_buffer)
            .expect("Failed to read buffer");

        assert_eq!(
            read_buffer, test_buffer,
            "Read buffer should match written buffer"
        );

        // Test error handling with invalid address
        let invalid_addr = GuestPhysAddr::from_usize(crate::test_utils::MEMORY_LEN + 0x1000);
        let result: AxResult<u32> = translator.read_obj(invalid_addr);
        assert!(result.is_err(), "Reading from invalid address should fail");

        let result = translator.write_obj(invalid_addr, 42u32);
        assert!(result.is_err(), "Writing to invalid address should fail");
    }

    #[test]
    #[axin(decorator(mock_hal_test))]
    fn test_two_vm_isolation() {
        // Create two different translators to simulate two different VMs
        let vm1_translator =
            MockTranslator::new(PhysAddr::from_usize(0), crate::test_utils::MEMORY_LEN / 2); // Offset for VM1
        let vm2_translator = MockTranslator::new(
            PhysAddr::from_usize(crate::test_utils::MEMORY_LEN / 2),
            crate::test_utils::MEMORY_LEN,
        ); // Offset for VM2

        // Both VMs write to the same guest address but different host memory regions
        let guest_addr = GuestPhysAddr::from_usize(0x100);
        let vm1_data: u64 = 0xDEADBEEFCAFEBABE;
        let vm2_data: u64 = 0x1234567890ABCDEF;

        // VM1 writes its data
        vm1_translator
            .write_obj(guest_addr, vm1_data)
            .expect("VM1 failed to write data");

        // VM2 writes its data
        vm2_translator
            .write_obj(guest_addr, vm2_data)
            .expect("VM2 failed to write data");

        // Both VMs read back their own data - should be isolated
        let vm1_read: u64 = vm1_translator
            .read_obj(guest_addr)
            .expect("VM1 failed to read data");
        let vm2_read: u64 = vm2_translator
            .read_obj(guest_addr)
            .expect("VM2 failed to read data");

        // Verify isolation: each VM should read its own data
        assert_eq!(vm1_read, vm1_data, "VM1 should read its own data");
        assert_eq!(vm2_read, vm2_data, "VM2 should read its own data");
        assert_ne!(
            vm1_read, vm2_read,
            "VM1 and VM2 should have different data (isolation)"
        );

        // Test buffer operations with different patterns
        let buffer_addr = GuestPhysAddr::from_usize(0x200);
        let vm1_buffer = [0xAA; 16]; // Pattern for VM1
        let vm2_buffer = [0x55; 16]; // Pattern for VM2

        // Both VMs write their patterns
        vm1_translator
            .write_buffer(buffer_addr, &vm1_buffer)
            .expect("VM1 failed to write buffer");
        vm2_translator
            .write_buffer(buffer_addr, &vm2_buffer)
            .expect("VM2 failed to write buffer");

        // Read back and verify isolation
        let mut vm1_read_buffer = [0u8; 16];
        let mut vm2_read_buffer = [0u8; 16];

        vm1_translator
            .read_buffer(buffer_addr, &mut vm1_read_buffer)
            .expect("VM1 failed to read buffer");
        vm2_translator
            .read_buffer(buffer_addr, &mut vm2_read_buffer)
            .expect("VM2 failed to read buffer");

        assert_eq!(
            vm1_read_buffer, vm1_buffer,
            "VM1 should read its own buffer pattern"
        );
        assert_eq!(
            vm2_read_buffer, vm2_buffer,
            "VM2 should read its own buffer pattern"
        );
        assert_ne!(
            vm1_read_buffer, vm2_read_buffer,
            "VM buffers should be isolated"
        );

        // Test that VM1 cannot access VM2's address space (beyond its limit)
        let vm2_only_addr = GuestPhysAddr::from_usize(crate::test_utils::MEMORY_LEN / 2 + 0x100);
        let result: AxResult<u32> = vm1_translator.read_obj(vm2_only_addr);
        assert!(
            result.is_err(),
            "VM1 should not be able to access VM2's exclusive address space"
        );
    }

    #[test]
    #[axin(decorator(mock_hal_test))]
    fn test_cross_page_access() {
        let translator =
            MockTranslator::new(PhysAddr::from_usize(0), crate::test_utils::MEMORY_LEN);

        // Test cross-page buffer operations
        // Place buffer near page boundary to ensure it crosses pages (assuming 4K pages)
        let page_size = 4096;
        let cross_page_addr = GuestPhysAddr::from_usize(page_size - 8); // 8 bytes before page boundary
        let test_data = [
            0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x0E,
            0x0F, 0x10,
        ]; // 16 bytes (crosses page)

        // Verify the buffer actually crosses page boundary
        assert!(
            translator.crosses_page_boundary(cross_page_addr, test_data.len()),
            "Test buffer should cross page boundary"
        );

        // Write cross-page data
        translator
            .write_buffer(cross_page_addr, &test_data)
            .expect("Failed to write cross-page buffer");

        // Read cross-page data back
        let mut read_data = [0u8; 16];
        translator
            .read_buffer(cross_page_addr, &mut read_data)
            .expect("Failed to read cross-page buffer");

        assert_eq!(
            read_data, test_data,
            "Cross-page read should match written data"
        );

        // Test individual byte access across page boundary
        for (i, &expected_byte) in test_data.iter().enumerate() {
            let byte_addr = GuestPhysAddr::from_usize(cross_page_addr.as_usize() + i);
            let read_byte: u8 = translator
                .read_obj(byte_addr)
                .expect("Failed to read individual byte");
            assert_eq!(
                read_byte, expected_byte,
                "Byte at offset {} should match",
                i
            );
        }
    }

    #[test]
    #[axin(decorator(mock_hal_test))]
    fn test_page_boundary_edge_cases() {
        let translator =
            MockTranslator::new(PhysAddr::from_usize(0), crate::test_utils::MEMORY_LEN);

        let page_size = 4096;

        // Test exactly at page boundary
        let page_boundary_addr = GuestPhysAddr::from_usize(page_size);
        let boundary_data = [0xAB, 0xCD, 0xEF, 0x12];

        translator
            .write_buffer(page_boundary_addr, &boundary_data)
            .expect("Failed to write at page boundary");

        let mut read_boundary = [0u8; 4];
        translator
            .read_buffer(page_boundary_addr, &mut read_boundary)
            .expect("Failed to read at page boundary");

        assert_eq!(
            read_boundary, boundary_data,
            "Page boundary data should match"
        );

        // Test zero-size buffer (should not cross any boundary)
        let empty_buffer: &[u8] = &[];
        translator
            .write_buffer(page_boundary_addr, empty_buffer)
            .expect("Empty buffer write should succeed");

        let mut empty_read: &mut [u8] = &mut [];
        translator
            .read_buffer(page_boundary_addr, &mut empty_read)
            .expect("Empty buffer read should succeed");

        // Test single byte at page boundary (should not cross)
        let single_byte = [0x42];
        assert!(
            !translator.crosses_page_boundary(page_boundary_addr, 1),
            "Single byte should not cross page boundary"
        );

        translator
            .write_buffer(page_boundary_addr, &single_byte)
            .expect("Single byte write should succeed");
    }
}
