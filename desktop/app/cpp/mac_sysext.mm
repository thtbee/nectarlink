// SPDX-License-Identifier: GPL-3.0-or-later
//
// Activates or removes the NectarLink camera extension (a CoreMediaIO system
// extension inside NectarLink.app) through OSSystemExtensionManager.
//
// This runs in the app's own executable: macOS only honors the restricted
// com.apple.developer.system-extension.install entitlement, backed by the
// app's provisioning profile, for a bundle's main executable. A separate
// helper binary next to it is killed at launch.

#import <Foundation/Foundation.h>
#import <Security/Security.h>
#import <SystemExtensions/SystemExtensions.h>

#include <stdio.h>

// Results of nl_sysext_request.
enum {
    NL_SYSEXT_DONE = 0,
    NL_SYSEXT_FAILED = 1,
    NL_SYSEXT_RESTART = 2, // done once the Mac restarts
    NL_SYSEXT_TIMEOUT = 3, // the user didn't approve it in time
};

typedef void (*nl_sysext_approval_fn)(void *ctx);

@interface NLSysextDelegate : NSObject <OSSystemExtensionRequestDelegate>
@property(nonatomic, strong) dispatch_semaphore_t done;
@property(nonatomic) int result;
@property(nonatomic) int errorCode;
@property(nonatomic, copy) NSString *errorText;
@property(nonatomic) nl_sysext_approval_fn onApproval;
@property(nonatomic) void *ctx;
@end

@implementation NLSysextDelegate

- (OSSystemExtensionReplacementAction)request:(OSSystemExtensionRequest *)request
                  actionForReplacingExtension:(OSSystemExtensionProperties *)existing
                                withExtension:(OSSystemExtensionProperties *)ext {
    return OSSystemExtensionReplacementActionReplace;
}

- (void)requestNeedsUserApproval:(OSSystemExtensionRequest *)request {
    if (self.onApproval) {
        self.onApproval(self.ctx);
    }
}

- (void)request:(OSSystemExtensionRequest *)request didFinishWithResult:(OSSystemExtensionRequestResult)result {
    self.result = result == OSSystemExtensionRequestWillCompleteAfterReboot ? NL_SYSEXT_RESTART : NL_SYSEXT_DONE;
    dispatch_semaphore_signal(self.done);
}

- (void)request:(OSSystemExtensionRequest *)request didFailWithError:(NSError *)error {
    self.result = NL_SYSEXT_FAILED;
    self.errorCode = [error.domain isEqualToString:OSSystemExtensionErrorDomain] ? (int)error.code : -1;
    self.errorText = error.localizedDescription;
    dispatch_semaphore_signal(self.done);
}

@end

// The first App Group this process is entitled to, from its own signature.
static NSString *own_app_group(void) {
    SecTaskRef task = SecTaskCreateFromSelf(NULL);
    if (task == NULL) {
        return nil;
    }
    CFTypeRef value = SecTaskCopyValueForEntitlement(task, CFSTR("com.apple.security.application-groups"), NULL);
    CFRelease(task);
    NSString *group = nil;
    if (value != NULL) {
        if (CFGetTypeID(value) == CFArrayGetTypeID() && CFArrayGetCount((CFArrayRef)value) > 0) {
            CFTypeRef first = CFArrayGetValueAtIndex((CFArrayRef)value, 0);
            if (CFGetTypeID(first) == CFStringGetTypeID()) {
                group = [NSString stringWithString:(__bridge NSString *)first];
            }
        }
        CFRelease(value);
    }
    return group;
}

// macOS creates an App Group's container when an entitled process first asks for
// it. The extension only starts when an app opens the camera, so create it up
// front: NectarLink writes the camera frames into it right after activation.
static void ensure_group_container(void) {
    NSString *group = own_app_group();
    if (group != nil) {
        (void)[[NSFileManager defaultManager] containerURLForSecurityApplicationGroupIdentifier:group];
    }
}

// Submits the request and waits: for the result, or for `timeout_secs`. Calls
// `on_approval(ctx)` from another thread if macOS needs the user's approval.
// On NL_SYSEXT_FAILED, `error_code` is the OSSystemExtensionError code (-1 when
// it isn't one) and `error_text` macOS's description.
extern "C" int nl_sysext_request(const char *bundle_id, int deactivate, int timeout_secs,
                                 nl_sysext_approval_fn on_approval, void *ctx, int *error_code,
                                 char *error_text, size_t error_text_len) {
    @autoreleasepool {
        NLSysextDelegate *delegate = [[NLSysextDelegate alloc] init];
        delegate.done = dispatch_semaphore_create(0);
        delegate.onApproval = on_approval;
        delegate.ctx = ctx;
        NSString *identifier = [NSString stringWithUTF8String:bundle_id];
        dispatch_queue_t queue = dispatch_queue_create("app.nectarlink.sysext", DISPATCH_QUEUE_SERIAL);
        OSSystemExtensionRequest *request =
            deactivate ? [OSSystemExtensionRequest deactivationRequestForExtension:identifier queue:queue]
                       : [OSSystemExtensionRequest activationRequestForExtension:identifier queue:queue];
        request.delegate = delegate;
        if (!deactivate) {
            ensure_group_container();
        }
        [[OSSystemExtensionManager sharedManager] submitRequest:request];
        dispatch_time_t deadline = dispatch_time(DISPATCH_TIME_NOW, (int64_t)timeout_secs * NSEC_PER_SEC);
        if (dispatch_semaphore_wait(delegate.done, deadline) != 0) {
            return NL_SYSEXT_TIMEOUT;
        }
        if (delegate.result == NL_SYSEXT_FAILED) {
            *error_code = delegate.errorCode;
            snprintf(error_text, error_text_len, "%s", delegate.errorText.UTF8String ?: "");
        }
        return delegate.result;
    }
}
