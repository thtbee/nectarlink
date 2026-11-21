// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.unlock

import android.annotation.SuppressLint
import android.content.Context
import android.os.Build
import android.os.SystemClock
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyInfo
import android.security.keystore.KeyPermanentlyInvalidatedException
import android.security.keystore.KeyProperties
import androidx.annotation.RequiresApi
import app.nectarlink.core.UnlockRefusal
import java.math.BigInteger
import java.security.KeyFactory
import java.security.KeyPairGenerator
import java.security.KeyStore
import java.security.MessageDigest
import java.security.PrivateKey
import java.security.Signature
import java.security.interfaces.ECPublicKey
import java.security.spec.ECGenParameterSpec
import java.security.spec.ECPoint
import java.util.UUID
import java.util.concurrent.CompletableFuture
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.asExecutor
import org.json.JSONObject

internal fun encodeLinuxUnlockPublicKey(point: ECPoint): ByteArray {
    fun coordinate(value: BigInteger): ByteArray {
        require(value.signum() >= 0 && value.bitLength() <= 256)
        val magnitude = value.toByteArray()
        val offset = if (magnitude.size == 33) 1 else 0
        val length = magnitude.size - offset
        return ByteArray(32).also { magnitude.copyInto(it, 32 - length, offset) }
    }
    return byteArrayOf(0x04) + coordinate(point.affineX) + coordinate(point.affineY)
}

internal object LinuxUnlockKeys {
    private const val STORE = "AndroidKeyStore"
    private const val PREFS = "linux_unlock_keys"
    private const val ALIAS = "nl-linux-unlock-"
    private val enrollmentProof = "nectarlink/linux-unlock/enrollment/v1\u0000".toByteArray(Charsets.US_ASCII)

    private fun pcPrefix(pc: String) = "$ALIAS$pc-"

    private fun accountPrefix(pc: String, uid: UInt) = "${pcPrefix(pc)}$uid-"

    private fun keyStore(): KeyStore = KeyStore.getInstance(STORE).apply { load(null) }

    private fun prefs(context: Context) = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)

    @RequiresApi(Build.VERSION_CODES.R)
    @SuppressLint("ApplySharedPref")
    fun enroll(context: Context, pc: String, pcName: String, userName: String, uid: UInt, timeoutMs: Long): ByteArray {
        if (timeoutMs <= 0) throw UnlockRefusal.Declined()
        val startedAt = SystemClock.elapsedRealtime()
        fun remainingMs(): Long {
            val remaining = timeoutMs - (SystemClock.elapsedRealtime() - startedAt)
            if (remaining <= 0) throw UnlockRefusal.Declined()
            return remaining
        }

        val account = accountPrefix(pc, uid)
        val confirmed = CompletableFuture<String?>()
        var stagedAlias: String? = null
        var staleAliases = emptyList<String>()
        try {
            val prefs = prefs(context)
            val previous: String?
            val (alias, signature, publicKey) = synchronized(this) {
                remainingMs()
                val store = keyStore()
                previous = prefs.getString(account, null)
                val record = previous?.let(::JSONObject)
                val existingAlias = record?.getString("alias")
                if (existingAlias != null && !existingAlias.startsWith(account)) throw UnlockRefusal.KeyGone()
                val existing = existingAlias?.let { reusableKey(store, it) }
                if (record != null && existingAlias != null && existing != null) {
                    if (record.getString("pc") != pcName || record.getString("user") != userName) {
                        throw UnlockRefusal.Declined()
                    }
                    Triple(existingAlias, existing.first, encodeLinuxUnlockPublicKey(existing.second.w))
                } else {
                    val alias = account + UUID.randomUUID()
                    stagedAlias = alias
                    val (signature, publicKey) = createKey(alias)
                    Triple(alias, signature, encodeLinuxUnlockPublicKey(publicKey.w))
                }
            }
            val authorized = UnlockPrompts.askToSign(context, pcName, userName, signature, remainingMs(), confirmed)
                ?: throw UnlockRefusal.Declined()
            val fingerprint = synchronized(this) {
                remainingMs()
                val store = keyStore()
                if (!store.containsAlias(alias) || prefs.getString(account, null) != previous) {
                    throw UnlockRefusal.KeyGone()
                }
                // Finish a separate signing operation before accepting the enrollment.
                authorized.update(enrollmentProof)
                authorized.sign()
                val fingerprint = MessageDigest.getInstance("SHA-256").digest(publicKey)
                    .joinToString("") { "%02x".format(it) }
                remainingMs()
                if (stagedAlias != null) {
                    staleAliases = store.aliases().toList().filter { it.startsWith(account) && it != alias }
                    val record = JSONObject().put("alias", alias).put("pc", pcName).put("user", userName)
                    remainingMs()
                    try {
                        if (!prefs.edit().putString(account, record.toString()).commit()) {
                            throw UnlockRefusal.Unsupported()
                        }
                        remainingMs()
                    } catch (err: Exception) {
                        prefs.edit().putString(account, previous).commit()
                        throw err
                    }
                    stagedAlias = null
                }
                fingerprint
            }
            confirmed.complete(fingerprint)
            if (staleAliases.isNotEmpty()) {
                // Aliases in this snapshot are never reused after committing their replacement.
                val aliasesToDelete = staleAliases
                runCatching {
                    CompletableFuture.runAsync({
                        val store = keyStore()
                        aliasesToDelete.forEach { runCatching { store.deleteEntry(it) } }
                    }, Dispatchers.IO.asExecutor())
                }
            }
            return publicKey
        } catch (err: Exception) {
            if (err is UnlockRefusal) throw err
            throw if (isInvalidated(err)) UnlockRefusal.KeyGone() else UnlockRefusal.Unsupported()
        } finally {
            stagedAlias?.let { runCatching { synchronized(this) { keyStore().deleteEntry(it) } } }
            confirmed.complete(null)
        }
    }

    private fun reusableKey(store: KeyStore, alias: String): Pair<Signature, ECPublicKey>? {
        return try {
            val entry = store.getEntry(alias, null) as? KeyStore.PrivateKeyEntry ?: return null
            signer(entry.privateKey) to (entry.certificate.publicKey as ECPublicKey)
        } catch (err: Exception) {
            if (isInvalidated(err)) null else throw err
        }
    }

    @RequiresApi(Build.VERSION_CODES.R)
    private fun createKey(alias: String): Pair<Signature, ECPublicKey> {
        val spec = KeyGenParameterSpec.Builder(alias, KeyProperties.PURPOSE_SIGN)
            .setAlgorithmParameterSpec(ECGenParameterSpec("secp256r1"))
            .setDigests(KeyProperties.DIGEST_SHA256)
            .setUserAuthenticationRequired(true)
            .setUserAuthenticationParameters(0, KeyProperties.AUTH_BIOMETRIC_STRONG)
            .setInvalidatedByBiometricEnrollment(true)
            .build()
        val key = KeyPairGenerator.getInstance(KeyProperties.KEY_ALGORITHM_EC, STORE).run {
            initialize(spec)
            generateKeyPair()
        }
        val info = KeyFactory.getInstance(KeyProperties.KEY_ALGORITHM_EC, STORE)
            .getKeySpec(key.private, KeyInfo::class.java)
        if (!info.isUserAuthenticationRequirementEnforcedBySecureHardware) throw UnlockRefusal.Unsupported()
        return signer(key.private) to (key.public as ECPublicKey)
    }

    @RequiresApi(Build.VERSION_CODES.R)
    fun sign(context: Context, pc: String, uid: UInt, transcript: ByteArray, timeoutMs: Long): ByteArray {
        val account = accountPrefix(pc, uid)
        val stored = prefs(context).getString(account, null) ?: throw UnlockRefusal.KeyGone()
        try {
            val record = JSONObject(stored)
            val alias = record.getString("alias")
            if (!alias.startsWith(account)) throw UnlockRefusal.KeyGone()
            val key = keyStore().getKey(alias, null) as? PrivateKey ?: throw UnlockRefusal.KeyGone()
            val authorized = UnlockPrompts.askToSign(
                context, record.getString("pc"), record.getString("user"), signer(key), timeoutMs,
            ) ?: throw UnlockRefusal.Declined()
            return synchronized(this) {
                if (prefs(context).getString(account, null) != stored) throw UnlockRefusal.KeyGone()
                authorized.update(transcript)
                authorized.sign()
            }
        } catch (err: Exception) {
            if (err is UnlockRefusal) throw err
            if (isInvalidated(err)) {
                synchronized(this) {
                    if (prefs(context).getString(account, null) == stored) forget(context, pc, uid)
                }
                throw UnlockRefusal.KeyGone()
            }
            throw UnlockRefusal.Unsupported()
        }
    }

    @Synchronized
    @SuppressLint("ApplySharedPref")
    fun forget(context: Context, pc: String, uid: UInt?) {
        val prefix = if (uid == null) pcPrefix(pc) else accountPrefix(pc, uid)
        val prefs = prefs(context)
        val edit = prefs.edit()
        prefs.all.keys.filter { it.startsWith(prefix) }.forEach(edit::remove)
        edit.commit()
        runCatching {
            val store = keyStore()
            store.aliases().toList().filter { it.startsWith(prefix) }.forEach { runCatching { store.deleteEntry(it) } }
        }
    }

    private fun signer(key: PrivateKey) = Signature.getInstance("SHA256withECDSA").apply { initSign(key) }

    private fun isInvalidated(err: Exception) =
        generateSequence<Throwable>(err) { it.cause }.any { it is KeyPermanentlyInvalidatedException }
}
