// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.core

import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import app.nectarlink.core.KeyProtector
import app.nectarlink.core.NectarlinkException
import java.security.KeyStore
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

/**
 * Encrypts the device's identity key with an AES-GCM key that lives in the
 * Android Keystore and can't be exported. Format: 12-byte IV, then the
 * ciphertext with its tag.
 */
class KeystoreKeyProtector : KeyProtector {

    override fun protect(plaintext: ByteArray): ByteArray = guard {
        val cipher = Cipher.getInstance(TRANSFORMATION)
        cipher.init(Cipher.ENCRYPT_MODE, key())
        cipher.iv + cipher.doFinal(plaintext)
    }

    override fun unprotect(ciphertext: ByteArray): ByteArray = guard {
        require(ciphertext.size > IV_LENGTH) { "protected key is too short" }
        val cipher = Cipher.getInstance(TRANSFORMATION)
        cipher.init(Cipher.DECRYPT_MODE, key(), GCMParameterSpec(TAG_BITS, ciphertext, 0, IV_LENGTH))
        cipher.doFinal(ciphertext, IV_LENGTH, ciphertext.size - IV_LENGTH)
    }

    private fun key(): SecretKey {
        val store = KeyStore.getInstance(KEYSTORE).apply { load(null) }
        (store.getKey(ALIAS, null) as? SecretKey)?.let { return it }
        val generator = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, KEYSTORE)
        generator.init(
            KeyGenParameterSpec.Builder(ALIAS, KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
                .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
                .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
                .setKeySize(256)
                .build(),
        )
        return generator.generateKey()
    }

    /** Reports Keystore failures to the core as errors instead of crashing. */
    private inline fun guard(block: () -> ByteArray): ByteArray = try {
        block()
    } catch (e: Exception) {
        throw NectarlinkException.Internal("keystore: ${e.javaClass.simpleName}: ${e.message}")
    }

    private companion object {
        const val KEYSTORE = "AndroidKeyStore"
        const val ALIAS = "nectarlink.identity.v1"
        const val TRANSFORMATION = "AES/GCM/NoPadding"
        const val IV_LENGTH = 12
        const val TAG_BITS = 128
    }
}
