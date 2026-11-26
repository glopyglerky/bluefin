package org.bluefin.android

import java.util.Base64
import javax.crypto.Cipher
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

/** Host-testable encryption used by AccountVault; Android supplies the nonexportable key. */
internal object ProtectedCleanupCodec {
    private val aad = "bluefin-account-v1".toByteArray(Charsets.UTF_8)
    data class Envelope(val iv: String, val body: String)
    fun seal(state: CleanupState, key: SecretKey): Envelope {
        val plain = state.json().toString()
        CleanupState.parse(plain)
        val cipher = Cipher.getInstance("AES/GCM/NoPadding").apply { init(Cipher.ENCRYPT_MODE, key) }
        cipher.updateAAD(aad)
        return Envelope(Base64.getEncoder().encodeToString(cipher.iv), Base64.getEncoder().encodeToString(cipher.doFinal(plain.toByteArray(Charsets.UTF_8))))
    }
    fun open(envelope: Envelope, key: SecretKey): String {
        require(envelope.iv.length <= 64 && envelope.body.length <= 48000) { "Protected account too large" }
        val iv = Base64.getDecoder().decode(envelope.iv)
        require(iv.size == 12)
        val cipher = Cipher.getInstance("AES/GCM/NoPadding").apply { init(Cipher.DECRYPT_MODE, key, GCMParameterSpec(128, iv)) }
        cipher.updateAAD(aad)
        val plain = String(cipher.doFinal(Base64.getDecoder().decode(envelope.body)), Charsets.UTF_8)
        require(plain.toByteArray(Charsets.UTF_8).size <= CleanupState.MAX_BYTES)
        return plain
    }
}
