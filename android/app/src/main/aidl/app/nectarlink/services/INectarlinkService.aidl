// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.services;

import android.os.ParcelFileDescriptor;
import app.nectarlink.services.IDisplaySession;
import app.nectarlink.services.INectarlinkServiceCallback;
import app.nectarlink.services.PointerScreenInfo;
import app.nectarlink.services.ServiceStatus;

interface INectarlinkService {
    int getApiVersion();
    ServiceStatus getStatus();
    void registerCallback(INectarlinkServiceCallback callback);
    void unregisterCallback(INectarlinkServiceCallback callback);
    boolean provisionAppPermissions(String targetPackage);

    boolean injectTouch(int displayId, int action, float x, float y);
    boolean injectScroll(int displayId, float x, float y, float dx, float dy);
    boolean injectKey(int displayId, int keyCode, int metaState);
    boolean injectText(int displayId, String text);
    boolean injectGamepad(int displayId, int buttons, int lx, int ly, int rx, int ry, int lt, int rt);

    PointerScreenInfo pointerStart(String edge, float y);
    boolean pointerMove(int dx, int dy, int buttons, int wheel);
    float[] pointerWhere();
    void pointerStop();

    IDisplaySession openScreenMirror(int width, int height, int dpi, int bitrate, int fps, String codec);
    IDisplaySession openAppDisplay(String pkg, int width, int height, int dpi, int bitrate, int fps, String codec);

    ParcelFileDescriptor openDeviceAudio();
    boolean isCallAudioSupported();
    ParcelFileDescriptor openCallAudio();

    boolean setScreenOff(boolean off);
    boolean wakeScreen();

    boolean setClipboard(String text, String html, boolean sensitive);
    boolean clearClipboard(String text);

    boolean expandNotifications();
    boolean setWifiEnabled(boolean enabled);
    boolean setBluetoothEnabled(boolean enabled);
    boolean setHotspotEnabled(boolean enabled);
    boolean setScreenBrightness(int brightness);
    boolean setMusicVolume(int index);
    boolean uninstallPackage(String pkg);
    boolean launchActivityFromBackground(String pkg, String activityClass);

    ParcelFileDescriptor openInputServerChannel();
}
