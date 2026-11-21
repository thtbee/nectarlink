// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.services

import android.os.Parcel
import android.os.Parcelable

data class ServiceStatus(
    val apiVersion: Int,
    val serviceEnabled: Boolean,
    val callerAuthorized: Boolean,
    val privilegedApp: Boolean,
    val capabilities: Long,
    val callAudioSupported: Boolean,
    val uhidAvailable: Boolean,
    val virtualDeviceSupported: Boolean,
) : Parcelable {
    fun hasCapability(flag: Long): Boolean = (capabilities and flag) == flag

    override fun describeContents(): Int = 0

    override fun writeToParcel(dest: Parcel, flags: Int) {
        dest.writeInt(apiVersion)
        dest.writeInt(if (serviceEnabled) 1 else 0)
        dest.writeInt(if (callerAuthorized) 1 else 0)
        dest.writeInt(if (privilegedApp) 1 else 0)
        dest.writeLong(capabilities)
        dest.writeInt(if (callAudioSupported) 1 else 0)
        dest.writeInt(if (uhidAvailable) 1 else 0)
        dest.writeInt(if (virtualDeviceSupported) 1 else 0)
    }

    companion object {
        @JvmField
        val CREATOR: Parcelable.Creator<ServiceStatus> = object : Parcelable.Creator<ServiceStatus> {
            override fun createFromParcel(source: Parcel): ServiceStatus = ServiceStatus(
                apiVersion = source.readInt(),
                serviceEnabled = source.readInt() != 0,
                callerAuthorized = source.readInt() != 0,
                privilegedApp = source.readInt() != 0,
                capabilities = source.readLong(),
                callAudioSupported = source.readInt() != 0,
                uhidAvailable = source.readInt() != 0,
                virtualDeviceSupported = source.readInt() != 0,
            )

            override fun newArray(size: Int): Array<ServiceStatus?> = arrayOfNulls(size)
        }
    }
}
