// SPDX-License-Identifier: GPL-3.0-or-later
// The macOS share picker for files: the Photos page's Share button (#174).

#import <AppKit/AppKit.h>

#include <cstddef>

extern "C" {

// Shows the share picker for the files at `paths` (UTF-8), over the app's
// window. Runs on the main thread, as the UI does. False when nothing could
// be shown.
bool mac_share_files(const char *const *paths, size_t count)
{
    @autoreleasepool {
        if (count == 0 || ![NSThread isMainThread])
            return false;
        NSMutableArray<NSURL *> *items = [NSMutableArray arrayWithCapacity:count];
        for (size_t i = 0; i < count; ++i) {
            NSString *path = paths[i] ? [NSString stringWithUTF8String:paths[i]] : nil;
            if (path.length > 0)
                [items addObject:[NSURL fileURLWithPath:path]];
        }
        if (items.count == 0)
            return false;
        NSWindow *window = NSApp.keyWindow ?: NSApp.mainWindow;
        NSView *view = window.contentView;
        if (!view)
            return false;
        // Kept until the next share: the picker has to outlive this call
        // while it's on screen.
        static NSSharingServicePicker *picker = nil;
        picker = [[NSSharingServicePicker alloc] initWithItems:items];
        NSRect bounds = view.bounds;
        NSRect anchor = NSMakeRect(NSMidX(bounds), NSMidY(bounds), 1, 1);
        [picker showRelativeToRect:anchor ofView:view preferredEdge:NSRectEdgeMinY];
        return true;
    }
}

} // extern "C"
