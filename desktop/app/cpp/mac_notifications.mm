// SPDX-License-Identifier: GPL-3.0-or-later
// macOS notifications through UserNotifications: action buttons, inline
// replies, in-place updates and click / dismiss events. Rust (mac/toast.rs)
// owns the state; this file only talks to the system.
//
// UserNotifications only works for a process that runs from an .app bundle
// with a bundle identifier. From a bare binary (`cargo run`) the first call
// into it raises an Objective-C exception, so every entry point checks
// `nl_notif_available()` first and Rust falls back to `osascript`.

#import <Foundation/Foundation.h>
#import <UserNotifications/UserNotifications.h>

#include <atomic>
#include <cstddef>
#include <cstdint>

extern "C" {

// kind: 0 opened (clicked the banner), 1 action button, 2 inline reply,
// 3 dismissed. `action` and `text` are empty when they don't apply.
typedef void (*NlNotifEventCallback)(
    int32_t kind,
    const char *device,
    const char *key,
    const char *action,
    const char *text);

} // extern "C"

namespace {

NSString *const kDeviceKey = @"nl.device";
NSString *const kKeyKey = @"nl.key";
NSString *const kPlainCategory = @"nl.plain";

NlNotifEventCallback g_callback = nullptr;
// -1 not known yet, 0 the user turned notifications off, 1 allowed.
std::atomic<int32_t> g_authorization{-1};

NSString *str(const char *value)
{
    if (!value)
        return @"";
    NSString *s = [NSString stringWithUTF8String:value];
    return s ? s : @"";
}

bool bundled()
{
    NSBundle *bundle = NSBundle.mainBundle;
    return bundle.bundleIdentifier.length > 0 && [bundle.bundleURL.pathExtension isEqualToString:@"app"];
}

void refresh_authorization()
{
    [[UNUserNotificationCenter currentNotificationCenter]
        getNotificationSettingsWithCompletionHandler:^(UNNotificationSettings *settings) {
            int32_t value = -1;
            switch (settings.authorizationStatus) {
            case UNAuthorizationStatusDenied:
                value = 0;
                break;
            case UNAuthorizationStatusNotDetermined:
                value = -1;
                break;
            default:
                // Allowed, but "None" for banners and the Notification Center
                // is as good as off.
                value = (settings.alertSetting == UNNotificationSettingDisabled
                            && settings.notificationCenterSetting == UNNotificationSettingDisabled)
                    ? 0
                    : 1;
                break;
            }
            g_authorization.store(value, std::memory_order_release);
        }];
}

NSMutableDictionary<NSString *, UNNotificationCategory *> *categories()
{
    static NSMutableDictionary<NSString *, UNNotificationCategory *> *registry = nil;
    static dispatch_once_t once;
    dispatch_once(&once, ^{
        registry = [NSMutableDictionary dictionary];
    });
    return registry;
}

// The category for this set of buttons, registered on first use. A category
// is fixed per notification, so each distinct set of ids gets its own.
NSString *category_for(NSArray<NSString *> *ids,
                       NSArray<NSString *> *titles,
                       NSString *reply_id,
                       NSString *reply_title)
{
    NSMutableString *identifier = [NSMutableString stringWithString:@"nl"];
    for (NSString *action_id in ids)
        [identifier appendFormat:@"|%@", action_id];
    if (reply_id.length > 0)
        [identifier appendFormat:@"~%@", reply_id];
    if (ids.count == 0 && reply_id.length == 0)
        return kPlainCategory;

    @synchronized(categories()) {
        if (categories()[identifier])
            return identifier;
        NSMutableArray<UNNotificationAction *> *actions = [NSMutableArray array];
        for (NSUInteger i = 0; i < ids.count; ++i) {
            [actions addObject:[UNNotificationAction actionWithIdentifier:ids[i]
                                                                    title:titles[i]
                                                                  options:UNNotificationActionOptionNone]];
        }
        if (reply_id.length > 0) {
            [actions addObject:[UNTextInputNotificationAction actionWithIdentifier:reply_id
                                                                              title:reply_title
                                                                            options:UNNotificationActionOptionNone
                                                               textInputButtonTitle:reply_title
                                                               textInputPlaceholder:reply_title]];
        }
        categories()[identifier] = [UNNotificationCategory categoryWithIdentifier:identifier
                                                                          actions:actions
                                                                intentIdentifiers:@[]
                                                                          options:UNNotificationCategoryOptionCustomDismissAction];
        [[UNUserNotificationCenter currentNotificationCenter]
            setNotificationCategories:[NSSet setWithArray:categories().allValues]];
    }
    return identifier;
}

NSString *identifier_for(NSString *device, NSString *key)
{
    return [NSString stringWithFormat:@"nl:%@:%@", device, key];
}

// A file the system may take: it moves an attachment into its own store, so
// hand it a copy and never the caller's file.
UNNotificationAttachment *attachment_for(NSString *path)
{
    if (path.length == 0)
        return nil;
    NSURL *source = [NSURL fileURLWithPath:path];
    NSString *name = [NSString stringWithFormat:@"nectarlink-%@.%@", NSUUID.UUID.UUIDString, source.pathExtension];
    NSURL *copy = [NSURL fileURLWithPath:[NSTemporaryDirectory() stringByAppendingPathComponent:name]];
    NSError *error = nil;
    if (![NSFileManager.defaultManager copyItemAtURL:source toURL:copy error:&error])
        return nil;
    UNNotificationAttachment *attachment = [UNNotificationAttachment attachmentWithIdentifier:@"picture"
                                                                                          URL:copy
                                                                                      options:nil
                                                                                        error:&error];
    if (!attachment)
        [NSFileManager.defaultManager removeItemAtURL:copy error:nil];
    return attachment;
}

} // namespace

@interface NectarLinkNotificationDelegate : NSObject <UNUserNotificationCenterDelegate>
@end

@implementation NectarLinkNotificationDelegate

// Show banners while the app is in front too: the phone's notifications are
// the point, whether or not NectarLink's window is open.
- (void)userNotificationCenter:(UNUserNotificationCenter *)center
       willPresentNotification:(UNNotification *)notification
         withCompletionHandler:(void (^)(UNNotificationPresentationOptions))completionHandler
{
    completionHandler(UNNotificationPresentationOptionBanner | UNNotificationPresentationOptionList
                      | UNNotificationPresentationOptionSound);
}

- (void)userNotificationCenter:(UNUserNotificationCenter *)center
    didReceiveNotificationResponse:(UNNotificationResponse *)response
             withCompletionHandler:(void (^)(void))completionHandler
{
    NSDictionary *info = response.notification.request.content.userInfo;
    NSString *device = info[kDeviceKey];
    NSString *key = info[kKeyKey];
    if (g_callback && device && key) {
        int32_t kind = 1;
        NSString *action = response.actionIdentifier;
        NSString *text = @"";
        if ([action isEqualToString:UNNotificationDefaultActionIdentifier]) {
            kind = 0;
            action = @"";
        } else if ([action isEqualToString:UNNotificationDismissActionIdentifier]) {
            kind = 3;
            action = @"";
        } else if ([response isKindOfClass:UNTextInputNotificationResponse.class]) {
            kind = 2;
            text = ((UNTextInputNotificationResponse *)response).userText;
        }
        g_callback(kind, device.UTF8String, key.UTF8String, action.UTF8String, text.UTF8String);
    }
    completionHandler();
}

@end

namespace {
NectarLinkNotificationDelegate *g_delegate = nil;
}

extern "C" {

// Whether this process can use UserNotifications at all (it runs from a
// bundle). When false, none of the other functions does anything.
bool nl_notif_available(void)
{
    return bundled();
}

// Takes the center's delegate and asks for permission (the system prompts the
// first time only). Returns false when not running from a bundle.
bool nl_notif_start(NlNotifEventCallback callback)
{
    if (!bundled())
        return false;
    g_callback = callback;
    g_delegate = [[NectarLinkNotificationDelegate alloc] init];
    UNUserNotificationCenter *center = UNUserNotificationCenter.currentNotificationCenter;
    center.delegate = g_delegate;
    [center requestAuthorizationWithOptions:(UNAuthorizationOptionAlert | UNAuthorizationOptionSound)
                          completionHandler:^(BOOL, NSError *) {
                              refresh_authorization();
                          }];
    return true;
}

// -1 not known yet, 0 off, 1 allowed. Starts a refresh, so the next call
// sees a change the user made in System Settings.
int32_t nl_notif_authorization(void)
{
    if (!bundled())
        return -1;
    refresh_authorization();
    return g_authorization.load(std::memory_order_acquire);
}

// Shows a notification, or replaces the one with the same device and key.
// `quiet` is for updating one that's already up (progress): no sound and no
// new banner.
void nl_notif_show(
    const char *device,
    const char *key,
    const char *title,
    const char *subtitle,
    const char *body,
    const char *attachment_path,
    const char *const *action_ids,
    const char *const *action_titles,
    size_t action_count,
    const char *reply_id,
    const char *reply_title,
    bool silent,
    bool call,
    bool quiet)
{
    if (!bundled())
        return;
    NSString *device_s = str(device);
    NSString *key_s = str(key);

    NSMutableArray<NSString *> *ids = [NSMutableArray arrayWithCapacity:action_count];
    NSMutableArray<NSString *> *titles = [NSMutableArray arrayWithCapacity:action_count];
    for (size_t i = 0; i < action_count; ++i) {
        [ids addObject:str(action_ids[i])];
        [titles addObject:str(action_titles[i])];
    }

    UNMutableNotificationContent *content = [[UNMutableNotificationContent alloc] init];
    content.title = str(title);
    content.subtitle = str(subtitle);
    content.body = str(body);
    content.threadIdentifier = device_s;
    content.userInfo = @{kDeviceKey : device_s, kKeyKey : key_s};
    content.categoryIdentifier = category_for(ids, titles, str(reply_id), str(reply_title));
    content.sound = (silent || quiet) ? nil : UNNotificationSound.defaultSound;
    // Quiet ones go to the Notification Center without a banner. A call stays
    // a normal, noticed notification even when it's "silent".
    if (@available(macOS 12.0, *)) {
        content.interruptionLevel = (quiet || (silent && !call)) ? UNNotificationInterruptionLevelPassive
                                                                 : UNNotificationInterruptionLevelActive;
    }
    UNNotificationAttachment *attachment = attachment_for(str(attachment_path));
    if (attachment)
        content.attachments = @[ attachment ];

    UNNotificationRequest *request = [UNNotificationRequest requestWithIdentifier:identifier_for(device_s, key_s)
                                                                          content:content
                                                                          trigger:nil];
    [UNUserNotificationCenter.currentNotificationCenter
        addNotificationRequest:request
         withCompletionHandler:^(NSError *error) {
             if (error)
                 refresh_authorization();
         }];
}

void nl_notif_remove(const char *device, const char *key)
{
    if (!bundled())
        return;
    NSArray<NSString *> *ids = @[ identifier_for(str(device), str(key)) ];
    UNUserNotificationCenter *center = UNUserNotificationCenter.currentNotificationCenter;
    [center removeDeliveredNotificationsWithIdentifiers:ids];
    [center removePendingNotificationRequestsWithIdentifiers:ids];
}

void nl_notif_remove_device(const char *device)
{
    if (!bundled())
        return;
    NSString *device_s = str(device);
    UNUserNotificationCenter *center = UNUserNotificationCenter.currentNotificationCenter;
    [center getDeliveredNotificationsWithCompletionHandler:^(NSArray<UNNotification *> *delivered) {
        NSMutableArray<NSString *> *ids = [NSMutableArray array];
        for (UNNotification *notification in delivered) {
            if ([notification.request.content.userInfo[kDeviceKey] isEqualToString:device_s])
                [ids addObject:notification.request.identifier];
        }
        if (ids.count > 0)
            [center removeDeliveredNotificationsWithIdentifiers:ids];
    }];
}

void nl_notif_remove_all(void)
{
    if (!bundled())
        return;
    UNUserNotificationCenter *center = UNUserNotificationCenter.currentNotificationCenter;
    [center removeAllDeliveredNotifications];
    [center removeAllPendingNotificationRequests];
}

} // extern "C"
