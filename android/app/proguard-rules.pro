# SPDX-License-Identifier: GPL-3.0-or-later
# JNA and the UniFFI bindings are reached through reflection and native code.
-keep class com.sun.jna.** { *; }
-keep class * implements com.sun.jna.** { *; }
-dontwarn java.awt.**
-keep class app.nectarlink.core.** { *; }
