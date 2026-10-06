// SPDX-License-Identifier: GPL-3.0-or-later
pluginManagement {
    repositories {
        google()
        mavenCentral()
        gradlePluginPortal()
    }
}

dependencyResolutionManagement {
    repositoriesMode.set(RepositoriesMode.FAIL_ON_PROJECT_REPOS)
    repositories {
        google()
        mavenCentral()
        // libadb-android (wireless debugging, for Elevated; GPL-3.0-or-later
        // or Apache-2.0) is published only there; nothing else may come
        // from it.
        exclusiveContent {
            forRepository { maven("https://jitpack.io") }
            filter {
                includeModule("com.github.MuntashirAkon", "libadb-android")
                // Its SPAKE2 (the pairing code's key exchange), LGPL-3.0.
                includeGroup("com.github.MuntashirAkon.spake2-java")
            }
        }
    }
}

rootProject.name = "Nectarlink"
include(":app")
