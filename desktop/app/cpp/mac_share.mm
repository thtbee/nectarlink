// SPDX-License-Identifier: GPL-3.0-or-later
// The Mac's share sheet (NSSharingServicePicker) for files: AirDrop, Messages, Mail and the
// other apps the system offers. Called on the main thread, which is Qt's.

#import <AppKit/AppKit.h>

extern "C" {

// Shows the share picker for the `count` files named by `paths` (UTF-8 paths) over the
// active window. 1 when shown, 0 when there's nothing to share or no window to show it on.
int nl_mac_share_files(const char *const *paths, int count)
{
    @autoreleasepool {
        NSMutableArray *items = [NSMutableArray array];
        for (int i = 0; i < count; i++) {
            NSString *path = paths[i] ? [NSString stringWithUTF8String:paths[i]] : nil;
            if (path.length > 0)
                [items addObject:[NSURL fileURLWithPath:path]];
        }
        if (items.count == 0)
            return 0;
        NSWindow *window = NSApp.keyWindow ?: NSApp.mainWindow;
        if (!window) {
            for (NSWindow *candidate in NSApp.windows) {
                if (candidate.isVisible && candidate.contentView) {
                    window = candidate;
                    break;
                }
            }
        }
        NSView *view = window.contentView;
        if (!view)
            return 0;
        // Held until the next one: the picker and the sheet it opens outlive this call.
        static NSSharingServicePicker *shown = nil;
        shown = [[NSSharingServicePicker alloc] initWithItems:items];
        NSRect bounds = view.bounds;
        NSRect anchor = NSMakeRect(NSMidX(bounds), NSMidY(bounds), 1, 1);
        [shown showRelativeToRect:anchor ofView:view preferredEdge:NSRectEdgeMinY];
        return 1;
    }
}

} // extern "C"
