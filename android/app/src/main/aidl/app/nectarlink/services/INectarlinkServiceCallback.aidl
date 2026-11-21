// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.services;

import app.nectarlink.services.ServiceStatus;

oneway interface INectarlinkServiceCallback {
    void onClipboardChanged(String text, String html, boolean isSensitive);
    void onInputBlocked();
    void onStatusChanged(in ServiceStatus status);
}
