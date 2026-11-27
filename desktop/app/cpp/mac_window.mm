// SPDX-License-Identifier: GPL-3.0-or-later
// What the floating windows (Nectar Island, the Shelf) need from AppKit: where
// the primary screen's usable area is, and the window behavior Qt doesn't give.

#import <AppKit/AppKit.h>

#include <cmath>
#include <cstdint>

extern "C" {

// The primary screen's work area, the screen minus the menu bar and the Dock,
// in physical pixels from the top left, as the other systems report it
// (QML divides by the device pixel ratio). False without a screen.
bool nl_mac_work_area(int32_t *x, int32_t *y, int32_t *width, int32_t *height)
{
    @autoreleasepool {
        // The first screen is the one with the menu bar.
        NSScreen *primary = NSScreen.screens.firstObject;
        if (!primary)
            return false;
        const NSRect frame = primary.frame;
        const NSRect usable = primary.visibleFrame;
        const CGFloat scale = primary.backingScaleFactor;
        // AppKit counts from the bottom left; Qt, and the callers, from the top left.
        *x = static_cast<int32_t>(std::lround((usable.origin.x - frame.origin.x) * scale));
        *y = static_cast<int32_t>(
            std::lround((NSMaxY(frame) - NSMaxY(usable)) * scale));
        *width = static_cast<int32_t>(std::lround(usable.size.width * scale));
        *height = static_cast<int32_t>(std::lround(usable.size.height * scale));
        return *width > 0 && *height > 0;
    }
}

// Makes this app's window titled `title` behave as an overlay: above other
// windows, on every Space, and still there when the app isn't the active one
// (a Qt tool window hides then, which a menu bar app always is). Qt sets
// these when it makes the window, so this runs once it's on screen. Not over
// full-screen apps, as on Windows, where the Island steps aside for them.
void nl_mac_style_overlay_window(const char *title)
{
    @autoreleasepool {
        NSString *wanted = [NSString stringWithUTF8String:title ? title : ""];
        for (NSWindow *window in NSApp.windows) {
            if (![window.title isEqualToString:wanted])
                continue;
            window.level = NSStatusWindowLevel;
            window.collectionBehavior = NSWindowCollectionBehaviorCanJoinAllSpaces
                | NSWindowCollectionBehaviorStationary | NSWindowCollectionBehaviorIgnoresCycle;
            window.hidesOnDeactivate = NO;
            window.canHide = NO;
            // The pill draws its own shadow.
            window.hasShadow = NO;
            // Hover (the Island opens as the pointer comes) and clicks reach it
            // while another app is in front, without making NectarLink the one.
            window.acceptsMouseMovedEvents = YES;
            if ([window isKindOfClass:NSPanel.class]) {
                NSPanel *panel = static_cast<NSPanel *>(window);
                panel.floatingPanel = YES;
                panel.becomesKeyOnlyIfNeeded = YES;
                panel.styleMask |= NSWindowStyleMaskNonactivatingPanel;
            }
        }
    }
}

} // extern "C"
