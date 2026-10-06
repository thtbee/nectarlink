# SPDX-License-Identifier: GPL-3.0-or-later
# JNA and the UniFFI bindings are reached through reflection and native code.
-keep class com.sun.jna.** { *; }
-keep class * implements com.sun.jna.** { *; }
-dontwarn java.awt.**
-keep class app.nectarlink.core.** { *; }
# Elevated: the input helper is started by name through app_process.
-keep class app.nectarlink.android.elevated.InputServer { public static void main(java.lang.String[]); }
# Conscrypt (TLS for wireless debugging): its native code finds classes by
# name, and its adapters for Android 4.x name classes that later Android
# doesn't have.
-keep class org.conscrypt.** { *; }
-dontwarn com.android.org.conscrypt.SSLParametersImpl
-dontwarn org.apache.harmony.xnet.provider.jsse.SSLParametersImpl
-keep class io.github.muntashirakon.** { *; }
