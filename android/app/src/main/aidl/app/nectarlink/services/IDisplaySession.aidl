// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.services;

import android.os.ParcelFileDescriptor;

interface IDisplaySession {
    ParcelFileDescriptor getVideoReadFd();
    int getDisplayId();
    void requestKeyframe();
    void resize(int width, int height, int dpi);
    void close();
}
