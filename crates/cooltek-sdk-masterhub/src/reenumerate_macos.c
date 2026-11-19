// Re-enumerates the USB device behind a hidapi macOS path (`DevSrvsID:<registry id>`), a software replug: the
// IOHIDDevice's parents are walked up to the first IOUSBHostDevice, which is opened through IOUSBLib, asked to
// re-enumerate and closed again. Nothing is sent to the module itself. Measured on 19 November 2026: an M4315 or
// M131E stuck in hardware mode comes back on a new HID path about 0.2 s later and takes software mode again. No
// root or entitlement is needed; without USBDeviceOpen the request fails with kIOReturnNotOpen.

#include <CoreFoundation/CoreFoundation.h>
#include <IOKit/IOCFPlugIn.h>
#include <IOKit/IOKitLib.h>
#include <IOKit/usb/IOUSBLib.h>
#include <stdint.h>
#include <stdio.h>

// Returns 0 when the re-enumeration was requested; otherwise 1 and a message in `error`.
int cooltek_reenumerate_usb_device(uint64_t registry_id, char *error, size_t error_length) {
    io_service_t entry =
        IOServiceGetMatchingService(kIOMainPortDefault, IORegistryEntryIDMatching(registry_id));
    if (entry == IO_OBJECT_NULL) {
        snprintf(error, error_length, "registry entry not found");
        return 1;
    }
    for (int i = 0; i < 8 && entry != IO_OBJECT_NULL && !IOObjectConformsTo(entry, "IOUSBHostDevice"); i++) {
        io_registry_entry_t parent = IO_OBJECT_NULL;
        kern_return_t kr = IORegistryEntryGetParentEntry(entry, kIOServicePlane, &parent);
        IOObjectRelease(entry);
        entry = kr == KERN_SUCCESS ? parent : IO_OBJECT_NULL;
    }
    if (entry == IO_OBJECT_NULL || !IOObjectConformsTo(entry, "IOUSBHostDevice")) {
        if (entry != IO_OBJECT_NULL) {
            IOObjectRelease(entry);
        }
        snprintf(error, error_length, "no IOUSBHostDevice above the HID service");
        return 1;
    }
    IOCFPlugInInterface **plugin = NULL;
    SInt32 score = 0;
    kern_return_t kr = IOCreatePlugInInterfaceForService(entry, kIOUSBDeviceUserClientTypeID,
                                                         kIOCFPlugInInterfaceID, &plugin, &score);
    IOObjectRelease(entry);
    if (kr != KERN_SUCCESS || plugin == NULL) {
        snprintf(error, error_length, "could not reach the USB device: 0x%08x", kr);
        return 1;
    }
    IOUSBDeviceInterface650 **device = NULL;
    HRESULT result = (*plugin)->QueryInterface(
        plugin, CFUUIDGetUUIDBytes(kIOUSBDeviceInterfaceID650), (LPVOID *)&device);
    (*plugin)->Release(plugin);
    if (result != S_OK || device == NULL) {
        snprintf(error, error_length, "the USB device interface is unavailable: 0x%08x", (int)result);
        return 1;
    }
    kr = (*device)->USBDeviceOpen(device);
    if (kr != KERN_SUCCESS) {
        (*device)->Release(device);
        snprintf(error, error_length, "could not open the USB device: 0x%08x", kr);
        return 1;
    }
    kr = (*device)->USBDeviceReEnumerate(device, 0);
    (*device)->USBDeviceClose(device);
    (*device)->Release(device);
    if (kr != KERN_SUCCESS) {
        snprintf(error, error_length, "the USB device refused to re-enumerate: 0x%08x", kr);
        return 1;
    }
    return 0;
}
