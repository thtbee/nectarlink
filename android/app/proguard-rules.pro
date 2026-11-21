# Keep class and method names readable in release APKs while still letting R8
# shrink unused code and resources, so crash stack traces from users read as
# they are, without a mapping file.
-dontobfuscate

# Preserve annotations (required by JNA @Structure.FieldOrder on UniFFI structs),
# generic signatures, inner classes ($ByValue / $ByReference), and line numbers.
-keepattributes *Annotation*,Signature,InnerClasses,EnclosingMethod,SourceFile,LineNumberTable

# UniFFI + JNA: JNA looks up Structure fields, Callback methods, and Library interfaces
# reflectively by name at runtime, and libjnidispatch.so reflects on JNA internals & Buffer.
-keep class com.sun.jna.** { *; }
-keep interface com.sun.jna.** { *; }
-keep class * extends com.sun.jna.Structure {
    public <init>(...);
    <fields>;
    <methods>;
}
-keep class * implements com.sun.jna.Structure$ByValue { *; }
-keep class * implements com.sun.jna.Structure$ByReference { *; }
-keep class * implements com.sun.jna.Callback { *; }
-keep class * implements com.sun.jna.Library { *; }
-keepclassmembers class java.nio.Buffer {
    long address;
    int capacity;
}
-dontwarn com.sun.jna.**
-dontwarn java.awt.**

# UniFFI generated bindings & callback interfaces (Platform, EventHandler, etc.)
-keep class uniffi.** { *; }
-keep interface uniffi.** { *; }
-keep class app.nectarlink.core.** { *; }
-keep interface app.nectarlink.core.** { *; }

# InputServer is invoked via app_process from adb/root shell (`app.nectarlink.android.elevated.InputServer`)
-keep class app.nectarlink.android.elevated.InputServer {
    public static void main(java.lang.String[]);
}
-keep class app.nectarlink.android.elevated.SurfaceControl { *; }
# Shizuku creates the Elevated service by name, with a Context (ShizukuShell.kt).
-keep class app.nectarlink.android.elevated.ShizukuShell {
    public <init>(android.content.Context);
}

# Conscrypt (used by Elevated wireless ADB TLS handshake)
-keep class org.conscrypt.** { *; }
-dontwarn org.conscrypt.**

# Custom ROM Nectarlink Services AIDL contract
-keep class app.nectarlink.services.** { *; }
