package org.tetherust.companion

import android.content.Context
import android.os.Build
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.util.Base64
import java.math.BigInteger
import java.security.*
import java.security.spec.ECGenParameterSpec
import java.util.Date
import javax.net.ssl.*
import javax.security.auth.x500.X500Principal
import org.json.JSONObject

object Identity {
    // A new alias migrates immutable RSA authorizations without deleting the old identity.
    // Android TLS signs prehashed input; DIGEST_NONE is required for that operation.
    private const val ALIAS = "tetherust-usb-tls-ec-v2"
    @Synchronized fun keyStore(context: Context): KeyStore {
        val store = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
        if (!store.containsAlias(ALIAS)) {
            val generator = KeyPairGenerator.getInstance(KeyProperties.KEY_ALGORITHM_EC, "AndroidKeyStore")
            val now = System.currentTimeMillis()
            generator.initialize(KeyGenParameterSpec.Builder(ALIAS, KeyProperties.PURPOSE_SIGN or KeyProperties.PURPOSE_VERIFY)
                .setAlgorithmParameterSpec(ECGenParameterSpec("secp256r1"))
                .setDigests(KeyProperties.DIGEST_NONE, KeyProperties.DIGEST_SHA256, KeyProperties.DIGEST_SHA384, KeyProperties.DIGEST_SHA512)
                .setCertificateSubject(X500Principal("CN=Tetherust Companion"))
                .setCertificateSerialNumber(BigInteger(63, SecureRandom())).setCertificateNotBefore(Date(now - 86400000L))
                .setCertificateNotAfter(Date(now + 10L * 365 * 86400000)).build())
            generator.generateKeyPair()
        }
        token(context)
        return store
    }
    @Synchronized fun token(context: Context): String {
        val prefs = context.getSharedPreferences("pairing", Context.MODE_PRIVATE)
        prefs.getString("token", null)?.let { return it }
        val bytes = ByteArray(32); SecureRandom().nextBytes(bytes)
        val token = bytes.joinToString("") { "%02x".format(it.toInt() and 255) }
        check(prefs.edit().putString("token", token).commit())
        return token
    }
    fun tls(context: Context): SSLContext {
        val store = keyStore(context)
        val manager = KeyManagerFactory.getInstance(KeyManagerFactory.getDefaultAlgorithm()).apply { init(store, null) }
        return SSLContext.getInstance("TLS").apply { init(manager.keyManagers, null, SecureRandom()) }
    }
    fun pairing(context: Context): String {
        val cert = keyStore(context).getCertificate(ALIAS).encoded
        return JSONObject().put("version",1).put("name", Build.MODEL.take(80)).put("token", token(context))
            .put("certificate", Base64.encodeToString(cert, Base64.NO_WRAP)).toString()
    }
    @Synchronized fun reset(context: Context) {
        val store = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }; for (alias in arrayOf(ALIAS, "tetherust-usb-tls-v1")) if (store.containsAlias(alias)) store.deleteEntry(alias)
        context.getSharedPreferences("pairing", Context.MODE_PRIVATE).edit().clear().commit()
    }
}
