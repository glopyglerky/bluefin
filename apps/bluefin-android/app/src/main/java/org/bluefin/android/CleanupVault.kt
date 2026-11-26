package org.bluefin.android

import android.content.Context
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import org.json.JSONObject
import java.io.File
import java.io.FileOutputStream
import java.security.KeyStore
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey

/** Version 2 state uses the existing AES-GCM Keystore key and authenticated AAD.
 * No token, server URL or operation IDs enter the interruption marker. Unreadable
 * ciphertext and the key are preserved for recovery, never cleared on load errors.
 */
internal class AccountVault(context: Context) : CleanupStore {
    private val prefs = context.getSharedPreferences("account-v1", Context.MODE_PRIVATE)
    private val marker = File(context.noBackupFilesDir.canonicalFile, "account-cleanup-interrupted-v2")
    private val alias = "bluefin-account-v1"
    private fun key(create: Boolean): SecretKey {
        val store = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
        (store.getKey(alias, null) as? SecretKey)?.let { return it }
        check(create) { "Protected account key unavailable" }
        return KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore").apply {
            init(KeyGenParameterSpec.Builder(alias, KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
                .setBlockModes(KeyProperties.BLOCK_MODE_GCM).setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE).build())
        }.generateKey()
    }
    override fun begin() {
        FileOutputStream(marker).use { it.write(byteArrayOf(2)); it.fd.sync() }
        syncDirectory()
    }
    override fun end() {
        check(!marker.exists() || marker.delete()) { "Protected sign-out marker could not be cleared" }
        syncDirectory()
    }
    private fun syncDirectory() {
        val fd = android.system.Os.open(marker.parent!!, android.system.OsConstants.O_RDONLY, 0)
        try { android.system.Os.fsync(fd) } finally { android.system.Os.close(fd) }
    }
    override fun write(state: CleanupState) {
        val sealed = ProtectedCleanupCodec.seal(state, key(true))
        check(prefs.edit().putString("iv", sealed.iv).putString("body", sealed.body).commit()) { "Protected account commit failed" }
    }
    override fun load(): CleanupState {
        val body = prefs.getString("body", null)
        if (body == null) {
            check(!prefs.contains("iv")) { "Incomplete protected account" }
            return CleanupState(pending = marker.exists())
        }
        val ivText = prefs.getString("iv", null) ?: error("Missing protected IV")
        val encoded = ProtectedCleanupCodec.open(ProtectedCleanupCodec.Envelope(ivText, body), key(false))
        val v = JSONObject(encoded)
        val state = if (v.has("version")) CleanupState.parse(encoded) else {
            // Exact legacy account-v1 migration. Never treat unknown schemas as an account.
            require(v.keys().asSequence().toSet() == setOf("server", "userId", "token", "deviceId"))
            CleanupState(Account(v.getString("server"), v.getString("userId"), v.getString("token"), v.getString("deviceId")).validate())
        }
        return if (marker.exists()) state.copy(pending = true) else state
    }
}
