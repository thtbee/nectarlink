// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.unlock

import android.content.Context
import android.hardware.biometrics.BiometricManager
import android.os.Build
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyPermanentlyInvalidatedException
import android.security.keystore.KeyProperties
import android.security.keystore.StrongBoxUnavailableException
import androidx.annotation.RequiresApi
import java.security.KeyStore
import java.security.MessageDigest
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

/**
 * The phone's half of unlocking a PC (docs/adr/0009-unlock-with-phone.md): one AES-256-GCM
 * key in Android Keystore per PC and Windows user. It never leaves the secure hardware,
 * every single use needs a strong biometric, and it's invalidated when a fingerprint is
 * added. It wraps the PC's key `K`; the PC keeps the result and sends it back for each
 * unlock.
 *
 * Wrapped format: version byte `1`, the 12-byte GCM IV, then the ciphertext and tag.
 */
object UnlockKeys {
    private const val STORE = "AndroidKeyStore"
    private const val PREFIX = "nl-unlock-"
    private const val FORMAT: Byte = 1
    private const val IV_LEN = 12
    private const val TAG_BITS = 128

    /** The key it's about is gone: added fingerprint, cleared data, wrong format. */
    class KeyGone : Exception()

    /** Whether this phone can keep keys behind a strong biometric (Android 11 and later). */
    fun supported(context: Context): Boolean =
        Build.VERSION.SDK_INT >= Build.VERSION_CODES.R &&
            context.getSystemService(BiometricManager::class.java)
                ?.canAuthenticate(BiometricManager.Authenticators.BIOMETRIC_STRONG) == BiometricManager.BIOMETRIC_SUCCESS

    private fun hash(s: String): String =
        MessageDigest.getInstance("SHA-256").digest(s.toByteArray()).take(8).joinToString("") { "%02x".format(it) }

    private fun pcPrefix(pc: String) = PREFIX + hash(pc) + "-"

    private fun alias(pc: String, user: String) = pcPrefix(pc) + hash(user)

    private fun keyStore(): KeyStore = KeyStore.getInstance(STORE).apply { load(null) }

    /**
     * A fresh key for [pc] and [user] (replacing any earlier one) and a cipher ready to wrap
     * with it once the owner's biometric authorizes it.
     */
    @RequiresApi(Build.VERSION_CODES.R)
    fun newWrapCipher(pc: String, user: String): Cipher {
        val alias = alias(pc, user)
        keyStore().deleteEntry(alias)
        val key = try {
            generate(alias, strongBox = Build.VERSION.SDK_INT >= Build.VERSION_CODES.P)
        } catch (_: StrongBoxUnavailableException) {
            generate(alias, strongBox = false)
        }
        return Cipher.getInstance("AES/GCM/NoPadding").apply { init(Cipher.ENCRYPT_MODE, key) }
    }

    @RequiresApi(Build.VERSION_CODES.R)
    private fun generate(alias: String, strongBox: Boolean): SecretKey {
        val spec = KeyGenParameterSpec.Builder(alias, KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
            .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
            .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
            .setKeySize(256)
            .setUserAuthenticationRequired(true)
            // 0: every use needs its own biometric, through BiometricPrompt's CryptoObject.
            .setUserAuthenticationParameters(0, KeyProperties.AUTH_BIOMETRIC_STRONG)
            .setInvalidatedByBiometricEnrollment(true)
            .setIsStrongBoxBacked(strongBox)
            .build()
        return KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, STORE).run {
            init(spec)
            generateKey()
        }
    }

    /** What the PC keeps, from a cipher the owner authorized and the key it wrapped. */
    fun wrapped(cipher: Cipher, ciphertext: ByteArray): ByteArray = byteArrayOf(FORMAT) + cipher.iv + ciphertext

    /**
     * A cipher ready to unwrap [wrapped] for [pc] and [user] once the owner's biometric
     * authorizes it. Throws [KeyGone] when the key isn't there or can't be used any more
     * (and deletes what's left of it).
     */
    @RequiresApi(Build.VERSION_CODES.R)
    fun unwrapCipher(pc: String, user: String, wrapped: ByteArray): Cipher {
        if (wrapped.size <= 1 + IV_LEN || wrapped[0] != FORMAT) throw KeyGone()
        val alias = alias(pc, user)
        val store = keyStore()
        val key = store.getKey(alias, null) as? SecretKey ?: throw KeyGone()
        return try {
            Cipher.getInstance("AES/GCM/NoPadding").apply {
                init(Cipher.DECRYPT_MODE, key, GCMParameterSpec(TAG_BITS, wrapped, 1, IV_LEN))
            }
        } catch (_: KeyPermanentlyInvalidatedException) {
            store.deleteEntry(alias)
            throw KeyGone()
        }
    }

    /** The wrapped key's ciphertext, past the format byte and IV. */
    fun ciphertext(wrapped: ByteArray): ByteArray = wrapped.copyOfRange(1 + IV_LEN, wrapped.size)

    /** Deletes the key for [pc] and [user], or every key for [pc]. */
    fun forget(pc: String, user: String?) {
        runCatching {
            val store = keyStore()
            if (user != null) {
                store.deleteEntry(alias(pc, user))
            } else {
                val prefix = pcPrefix(pc)
                store.aliases().toList().filter { it.startsWith(prefix) }.forEach(store::deleteEntry)
            }
        }
    }
}
