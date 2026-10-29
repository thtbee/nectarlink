// SPDX-License-Identifier: GPL-3.0-or-later
#import <Foundation/Foundation.h>
#import <SystemExtensions/SystemExtensions.h>
#import <Security/Security.h>
#include "NectarLinkCameraActivation.h"
#include "NectarLinkCamera.h"

static NSString *const extensionID = @"app.nectarlink.desktop.CameraExtension";
// Validate this signed host and its embedded, same-team extension. A directory
// or a public device UID alone cannot enable output or submit activation.
int32_t nl_camera_payload_valid(void) {
    @autoreleasepool {
        NSBundle *host = NSBundle.mainBundle;
        if (![host.bundleIdentifier isEqual:@NL_CAMERA_HOST_ID]) return 0;
        NSURL *url = [host.bundleURL URLByAppendingPathComponent:
            @"Contents/Library/SystemExtensions/" NL_CAMERA_EXTENSION_ID ".systemextension"];
        NSBundle *extension = [NSBundle bundleWithURL:url];
        if (![extension.bundleIdentifier isEqual:@NL_CAMERA_EXTENSION_ID]) return 0;
        SecCodeRef ownCode = NULL;
        CFDictionaryRef info = NULL;
        if (SecCodeCopySelf(kSecCSDefaultFlags, &ownCode) != errSecSuccess) return 0;
        SecStaticCodeRef ownStatic = NULL;
        OSStatus status = SecCodeCopyStaticCode(ownCode, kSecCSDefaultFlags, &ownStatic);
        if (status == errSecSuccess)
            status = SecCodeCopySigningInformation(ownStatic, kSecCSSigningInformation, &info);
        if (ownStatic) CFRelease(ownStatic);
        NSString *team = info ? ((__bridge NSDictionary *)info)[(__bridge id)kSecCodeInfoTeamIdentifier] : nil;
        if (status != errSecSuccess || ![team isKindOfClass:NSString.class] || team.length != 10 ||
            [team rangeOfCharacterFromSet:[[NSCharacterSet characterSetWithCharactersInString:
                @"ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789"] invertedSet]].location != NSNotFound) {
            if (info) CFRelease(info);
            CFRelease(ownCode); return 0;
        }
        NSString *hostRule = [NSString stringWithFormat:
            @"anchor apple generic and identifier \"%s\" and certificate leaf[subject.OU] = \"%@\"", NL_CAMERA_HOST_ID, team];
        NSString *extensionRule = [NSString stringWithFormat:
            @"anchor apple generic and identifier \"%s\" and certificate leaf[subject.OU] = \"%@\"", NL_CAMERA_EXTENSION_ID, team];
        CFRelease(info);
        SecRequirementRef requirement = NULL;
        status = SecRequirementCreateWithString((__bridge CFStringRef)hostRule, kSecCSDefaultFlags, &requirement);
        if (status == errSecSuccess) status = SecCodeCheckValidity(ownCode, kSecCSStrictValidate, requirement);
        if (requirement) CFRelease(requirement);
        CFRelease(ownCode);
        if (status != errSecSuccess) return 0;
        SecStaticCodeRef code = NULL;
        requirement = NULL;
        status = SecStaticCodeCreateWithPath((__bridge CFURLRef)url, kSecCSDefaultFlags, &code);
        if (status == errSecSuccess) status = SecRequirementCreateWithString(
            (__bridge CFStringRef)extensionRule, kSecCSDefaultFlags, &requirement);
        if (status == errSecSuccess) status = SecStaticCodeCheckValidity(code,
            kSecCSStrictValidate | kSecCSCheckAllArchitectures, requirement);
        if (requirement) CFRelease(requirement);
        if (code) CFRelease(code);
        return status == errSecSuccess ? 1 : 0;
    }
}

@interface NLActivation : NSObject <OSSystemExtensionRequestDelegate>
@property(nonatomic, strong) OSSystemExtensionRequest *request;
@property(nonatomic) NLCameraActivationCallback callback;
@property(nonatomic) void *context;
@end
@implementation NLActivation
- (void)requestNeedsUserApproval:(OSSystemExtensionRequest *)request {
    if (request == self.request && self.callback)
        self.callback(self.context, NL_CAMERA_ACTIVATION_NEEDS_APPROVAL, 0);
}
- (OSSystemExtensionReplacementAction)request:(OSSystemExtensionRequest *)request
                actionForReplacingExtension:(OSSystemExtensionProperties *)existing
                              withExtension:(OSSystemExtensionProperties *)replacement {
    if (request != self.request || ![existing.bundleIdentifier isEqual:extensionID] ||
        ![replacement.bundleIdentifier isEqual:extensionID]) return OSSystemExtensionReplacementActionCancel;
    return [replacement.bundleVersion compare:existing.bundleVersion options:NSNumericSearch] != NSOrderedAscending
        ? OSSystemExtensionReplacementActionReplace : OSSystemExtensionReplacementActionCancel;
}
- (void)request:(OSSystemExtensionRequest *)request didFinishWithResult:(OSSystemExtensionRequestResult)result {
    if (request != self.request) return;
    NLCameraActivationCallback callback = self.callback; void *context = self.context;
    self.request = nil; self.callback = NULL; self.context = NULL;
    if (callback) callback(context, result == OSSystemExtensionRequestCompleted
        ? NL_CAMERA_ACTIVATION_COMPLETED : NL_CAMERA_ACTIVATION_REBOOT, 0);
}
- (void)request:(OSSystemExtensionRequest *)request didFailWithError:(NSError *)error {
    if (request != self.request) return;
    NLCameraActivationCallback callback = self.callback; void *context = self.context;
    self.request = nil; self.callback = NULL; self.context = NULL;
    if (callback) callback(context, NL_CAMERA_ACTIVATION_FAILED, error.code);
}
@end

void nl_camera_request_activation(int32_t enable, NLCameraActivationCallback callback, void *context) {
    dispatch_async(dispatch_get_main_queue(), ^{
        static NLActivation *activation;
        if (!activation) activation = [NLActivation new];
        if (activation.request) {
            if (callback) callback(context, NL_CAMERA_ACTIVATION_BUSY, 0);
            return;
        }
        if (nl_camera_payload_valid() != 1) {
            if (callback) callback(context, NL_CAMERA_ACTIVATION_FAILED, -1);
            return;
        }
        activation.callback = callback; activation.context = context;
        activation.request = enable
            ? [OSSystemExtensionRequest activationRequestForExtension:extensionID queue:dispatch_get_main_queue()]
            : [OSSystemExtensionRequest deactivationRequestForExtension:extensionID queue:dispatch_get_main_queue()];
        activation.request.delegate = activation;
        if (callback) callback(context, NL_CAMERA_ACTIVATION_SUBMITTED, 0);
        [[OSSystemExtensionManager sharedManager] submitRequest:activation.request];
    });
}
