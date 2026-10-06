// SPDX-License-Identifier: GPL-3.0-or-later
import javax.inject.Inject

plugins {
    alias(libs.plugins.android.application)
    alias(libs.plugins.kotlin.compose)
}

// The version comes from the workspace (Cargo.toml), like the PC app's.
val appVersion: String = rootProject.file("../Cargo.toml").readLines()
    .first { it.startsWith("version = ") }
    .substringAfter('"').substringBefore('"')
/** 1.2.3 → 1002003: every release must have a larger code than the last. */
val appVersionCode: Int = appVersion.substringBefore('-').split('.').map(String::toInt)
    .let { (major, minor, patch) -> major * 1_000_000 + minor * 1_000 + patch }

android {
    namespace = "app.nectarlink.android"
    compileSdk = 37
    ndkVersion = "30.0.16248370"

    defaultConfig {
        applicationId = "app.nectarlink"
        minSdk = 26
        targetSdk = 37
        versionCode = appVersionCode
        versionName = appVersion
    }

    // Release builds are signed with the project's key when CI provides it
    // (NECTARLINK_KEYSTORE: a .jks file; NECTARLINK_KEYSTORE_PASSWORD).
    // Without it, a release build comes out unsigned.
    val keystore = System.getenv("NECTARLINK_KEYSTORE")?.let(::file)?.takeIf { it.exists() }
    signingConfigs {
        if (keystore != null) {
            create("release") {
                storeFile = keystore
                storePassword = System.getenv("NECTARLINK_KEYSTORE_PASSWORD")
                keyAlias = "nectarlink"
                keyPassword = System.getenv("NECTARLINK_KEYSTORE_PASSWORD")
            }
        }
    }

    buildTypes {
        release {
            if (keystore != null) signingConfig = signingConfigs.getByName("release")
            isMinifyEnabled = true
            isShrinkResources = true
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    buildFeatures {
        compose = true
        buildConfig = true
    }

}

// ---- The Rust core (nectarlink-ffi) ----
//
// Built with cargo-ndk for the ABIs in `nectarlink.abis` (default: devices
// and the x86_64 emulator), then UniFFI generates the Kotlin bindings from
// the built library. Both are registered as generated sources, so AGP runs
// them when needed. Rust is always built in release: debug builds of the
// core are large and slow.

/** Builds the Rust core into `<outputDir>/<abi>/libnectarlink_ffi.so`. */
abstract class CargoNdkBuild : DefaultTask() {
    @get:Inject abstract val exec: ExecOperations
    @get:Input abstract val abis: ListProperty<String>
    @get:InputDirectory @get:PathSensitive(PathSensitivity.RELATIVE) abstract val crates: DirectoryProperty
    @get:InputFile @get:PathSensitive(PathSensitivity.NONE) abstract val lockFile: RegularFileProperty
    @get:Internal abstract val workspace: DirectoryProperty
    @get:Internal abstract val ndk: DirectoryProperty
    @get:OutputDirectory abstract val outputDir: DirectoryProperty

    @TaskAction
    fun build() {
        exec.exec {
            workingDir = workspace.get().asFile
            environment("ANDROID_NDK_HOME", ndk.get().asFile.absolutePath)
            commandLine(
                buildList {
                    addAll(listOf("cargo", "ndk", "--platform", "26", "-o", outputDir.get().asFile.path))
                    abis.get().forEach { addAll(listOf("-t", it)) }
                    addAll(listOf("build", "--profile", "android", "--locked", "-p", "nectarlink-ffi"))
                },
            )
        }
    }
}

/** Generates the Kotlin bindings from a built core library. */
abstract class UniffiBindgen : DefaultTask() {
    @get:Inject abstract val exec: ExecOperations
    @get:InputFile @get:PathSensitive(PathSensitivity.NONE) abstract val library: RegularFileProperty
    @get:Internal abstract val workspace: DirectoryProperty
    @get:OutputDirectory abstract val outputDir: DirectoryProperty

    @TaskAction
    fun generate() {
        exec.exec {
            workingDir = workspace.get().asFile
            commandLine(
                "cargo", "run", "--quiet", "--locked", "-p", "nectarlink-ffi", "--features", "bindgen",
                "--bin", "uniffi-bindgen", "--", "generate", "--library", library.get().asFile.path,
                "--language", "kotlin", "--no-format", "--out-dir", outputDir.get().asFile.path,
            )
        }
    }
}

// ---- Fonts ----
//
// The shared fonts in `assets/fonts` (with their licenses) are packaged as
// app assets under `fonts/`.

/** Copies the bundled fonts and their licenses into `<outputDir>/fonts`. */
abstract class BundleFonts : DefaultTask() {
    @get:Inject abstract val fs: FileSystemOperations
    @get:InputDirectory @get:PathSensitive(PathSensitivity.RELATIVE) abstract val fonts: DirectoryProperty
    @get:OutputDirectory abstract val outputDir: DirectoryProperty

    @TaskAction
    fun copy() {
        fs.sync {
            from(fonts) { include("*.ttf", "OFL-*.txt") }
            into(outputDir.dir("fonts"))
        }
    }
}

val workspaceDir: Directory = rootProject.layout.projectDirectory.dir("..")
val rustAbis: List<String> =
    (findProperty("nectarlink.abis") as String?)?.split(",")?.map(String::trim) ?: listOf("arm64-v8a", "x86_64")

androidComponents {
    onVariants { variant ->
        val suffix = variant.name.replaceFirstChar(Char::uppercase)
        val cargo = tasks.register<CargoNdkBuild>("cargoNdkBuild$suffix") {
            description = "Builds the Rust core for Android."
            abis.set(rustAbis)
            crates.set(workspaceDir.dir("core"))
            lockFile.set(workspaceDir.file("Cargo.lock"))
            workspace.set(workspaceDir)
            ndk.set(androidComponents.sdkComponents.ndkDirectory)
        }
        val bindgen = tasks.register<UniffiBindgen>("uniffiBindgen$suffix") {
            description = "Generates the Kotlin bindings for the Rust core."
            library.set(cargo.flatMap { it.outputDir.file("${rustAbis.first()}/libnectarlink_ffi.so") })
            workspace.set(workspaceDir)
        }
        val fonts = tasks.register<BundleFonts>("bundleFonts$suffix") {
            description = "Packages the shared fonts as assets."
            this.fonts.set(workspaceDir.dir("assets/fonts"))
        }
        variant.sources.jniLibs?.addGeneratedSourceDirectory(cargo, CargoNdkBuild::outputDir)
        variant.sources.java?.addGeneratedSourceDirectory(bindgen, UniffiBindgen::outputDir)
        variant.sources.assets?.addGeneratedSourceDirectory(fonts, BundleFonts::outputDir)
    }
}

dependencies {
    implementation(platform(libs.compose.bom))
    implementation(libs.compose.ui)
    implementation(libs.compose.material3)
    implementation(libs.compose.ui.tooling.preview)
    debugImplementation(libs.compose.ui.tooling)
    implementation(libs.androidx.core.ktx)
    implementation(libs.androidx.activity.compose)
    implementation(libs.androidx.lifecycle.runtime.compose)
    implementation(libs.androidx.lifecycle.viewmodel.compose)
    implementation(libs.androidx.lifecycle.service)
    implementation(libs.kotlinx.coroutines.android)
    implementation(libs.camerax.camera2)
    implementation(libs.camerax.lifecycle)
    implementation(libs.camerax.view)
    implementation(libs.zxing.core)
    // Elevated: the phone's own wireless debugging (pairing and shell).
    implementation(libs.libadb.android)
    implementation(libs.conscrypt.android)
    // UniFFI's Kotlin bindings call the core through JNA.
    implementation(libs.jna) { artifact { type = "aar" } }
    testImplementation(libs.junit)
}
