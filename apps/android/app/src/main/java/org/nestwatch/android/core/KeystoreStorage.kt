// SPDX-License-Identifier: AGPL-3.0-or-later
package org.nestwatch.android.core

import android.content.Context
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.util.AtomicFile
import org.nestwatch.ffi.Storage
import org.nestwatch.ffi.StorageFailure
import java.io.File
import java.io.FileNotFoundException
import java.security.KeyStore
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

/**
 * The engines' storage port on Android (security.md §5): every value is
 * sealed with AES-256-GCM under a non-exportable Android Keystore key
 * (API 23+) and written atomically to `noBackupFilesDir/nestwatch/<name>.sealed`,
 * which Android never includes in backups (the manifest also disables
 * backups and device transfer).
 *
 * File layout: version byte `1`, 12-byte IV, ciphertext with 16-byte tag.
 * The storage name is the AAD, so a file renamed to another key fails to
 * open. Values are never logged.
 */
class KeystoreStorage(context: Context) : Storage {
    private val dir = File(context.noBackupFilesDir, "nestwatch").apply { mkdirs() }
    private val lock = Any()

    private fun file(key: String): File {
        require(NAME.matches(key) && !key.startsWith('.')) { "invalid storage key" }
        return File(dir, "$key.sealed")
    }

    private fun secretKey(): SecretKey {
        val ks = KeyStore.getInstance(KEYSTORE).apply { load(null) }
        (ks.getKey(ALIAS, null) as? SecretKey)?.let { return it }
        val gen = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, KEYSTORE)
        gen.init(
            KeyGenParameterSpec.Builder(ALIAS, KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
                .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
                .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
                .setKeySize(256)
                .setRandomizedEncryptionRequired(true)
                .build(),
        )
        return gen.generateKey()
    }

    override fun get(key: String): ByteArray? = guard {
        val f = file(key)
        val data = try {
            AtomicFile(f).readFully()
        } catch (_: FileNotFoundException) {
            return@guard null
        }
        if (data.size < 1 + IV_LEN + TAG_LEN || data[0] != VERSION) {
            throw StorageFailure.Backend("sealed value $key has an unknown format")
        }
        val cipher = Cipher.getInstance(TRANSFORMATION)
        cipher.init(Cipher.DECRYPT_MODE, secretKey(), GCMParameterSpec(TAG_LEN * 8, data, 1, IV_LEN))
        cipher.updateAAD(key.toByteArray(Charsets.US_ASCII))
        cipher.doFinal(data, 1 + IV_LEN, data.size - 1 - IV_LEN)
    }

    override fun put(key: String, value: ByteArray) = guard {
        val cipher = Cipher.getInstance(TRANSFORMATION)
        cipher.init(Cipher.ENCRYPT_MODE, secretKey())
        cipher.updateAAD(key.toByteArray(Charsets.US_ASCII))
        val sealed = cipher.doFinal(value)
        val iv = cipher.iv
        check(iv.size == IV_LEN) { "unexpected IV length" }
        val af = AtomicFile(file(key))
        val out = af.startWrite()
        try {
            out.write(byteArrayOf(VERSION))
            out.write(iv)
            out.write(sealed)
            af.finishWrite(out)
        } catch (e: Exception) {
            af.failWrite(out)
            throw e
        }
    }

    override fun delete(key: String) = guard {
        AtomicFile(file(key)).delete()
    }

    private inline fun <T> guard(block: () -> T): T = synchronized(lock) {
        try {
            block()
        } catch (e: StorageFailure) {
            throw e
        } catch (e: Exception) {
            // The message names the failing operation, never the value.
            throw StorageFailure.Backend(e.javaClass.simpleName + ": " + (e.message ?: ""))
        }
    }

    companion object {
        private const val KEYSTORE = "AndroidKeyStore"
        private const val ALIAS = "org.nestwatch.storage.v1"
        private const val TRANSFORMATION = "AES/GCM/NoPadding"
        private const val VERSION: Byte = 1
        private const val IV_LEN = 12
        private const val TAG_LEN = 16
        private val NAME = Regex("[a-z0-9._-]{1,64}")
    }
}
