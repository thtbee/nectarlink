// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.services

import android.os.Parcel
import android.os.Parcelable

data class PointerScreenInfo(
    val width: Int,
    val height: Int,
    val scale: Float,
) : Parcelable {
    override fun describeContents(): Int = 0

    override fun writeToParcel(dest: Parcel, flags: Int) {
        dest.writeInt(width)
        dest.writeInt(height)
        dest.writeFloat(scale)
    }

    companion object {
        @JvmField
        val CREATOR: Parcelable.Creator<PointerScreenInfo> = object : Parcelable.Creator<PointerScreenInfo> {
            override fun createFromParcel(source: Parcel): PointerScreenInfo = PointerScreenInfo(
                width = source.readInt(),
                height = source.readInt(),
                scale = source.readFloat(),
            )

            override fun newArray(size: Int): Array<PointerScreenInfo?> = arrayOfNulls(size)
        }
    }
}
