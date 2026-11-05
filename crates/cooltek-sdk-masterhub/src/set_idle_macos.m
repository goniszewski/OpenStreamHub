// Sends one HID class request, SET_IDLE(0) to interface 0, to the IOUSBHostDevice behind a
// hidapi macOS path (`DevSrvsID:<registry id>`). The device is opened WITHOUT capture
// (IOUSBHostObjectInitOptionsNone), so the system HID driver keeps the interface. Nothing
// else is ever sent. See GitHub issue #4.

#import <Foundation/Foundation.h>
#import <IOKit/IOKitLib.h>
#import <IOUSBHost/IOUSBHost.h>
#include <stdint.h>
#include <stdio.h>

static void set_error(char *buffer, size_t length, NSString *message) {
    if (buffer && length) {
        snprintf(buffer, length, "%s", [message UTF8String]);
    }
}

// Returns 0 when the device accepted SET_IDLE(0); otherwise 1 and a message in `error`.
int cooltek_roller_set_idle(uint64_t registry_id, char *error, size_t error_length) {
    @try {
        @autoreleasepool {
            io_service_t entry = IOServiceGetMatchingService(
                kIOMainPortDefault, IORegistryEntryIDMatching(registry_id));
            if (entry == IO_OBJECT_NULL) {
                set_error(error, error_length, @"registry entry not found");
                return 1;
            }
            for (int i = 0; i < 8 && entry != IO_OBJECT_NULL
                            && !IOObjectConformsTo(entry, "IOUSBHostDevice"); i++) {
                io_registry_entry_t parent = IO_OBJECT_NULL;
                kern_return_t kr = IORegistryEntryGetParentEntry(entry, kIOServicePlane, &parent);
                IOObjectRelease(entry);
                entry = kr == KERN_SUCCESS ? parent : IO_OBJECT_NULL;
            }
            if (entry == IO_OBJECT_NULL || !IOObjectConformsTo(entry, "IOUSBHostDevice")) {
                if (entry != IO_OBJECT_NULL) {
                    IOObjectRelease(entry);
                }
                set_error(error, error_length, @"no IOUSBHostDevice above the HID service");
                return 1;
            }
            NSError *ns_error = nil;
            IOUSBHostDevice *device = [[IOUSBHostDevice alloc] initWithIOService:entry
                                                                         options:IOUSBHostObjectInitOptionsNone
                                                                           queue:nil
                                                                           error:&ns_error
                                                                  interestHandler:nil];
            IOObjectRelease(entry);
            if (!device) {
                set_error(error, error_length,
                          [NSString stringWithFormat:@"could not open the USB device: %@", ns_error]);
                return 1;
            }
            IOUSBDeviceRequest request = { .bmRequestType = 0x21, .bRequest = 0x0A,
                                           .wValue = 0, .wIndex = 0, .wLength = 0 };
            NSUInteger transferred = 0;
            BOOL ok = [device sendDeviceRequest:request
                                           data:nil
                                bytesTransferred:&transferred
                              completionTimeout:2.0
                                          error:&ns_error];
            [device destroy];
            if (!ok) {
                set_error(error, error_length,
                          [NSString stringWithFormat:@"SET_IDLE request failed: %@", ns_error]);
                return 1;
            }
            return 0;
        }
    } @catch (NSException *exception) {
        set_error(error, error_length,
                  [NSString stringWithFormat:@"exception: %@", exception.reason]);
        return 1;
    }
}
