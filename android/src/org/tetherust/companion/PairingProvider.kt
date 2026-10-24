package org.tetherust.companion

import android.content.*
import android.database.Cursor
import android.net.Uri
import android.os.*
import java.io.FileNotFoundException

/** Pairing is exported ONLY to an already authorized ADB shell, never arbitrary installed apps.
 * This deliberately trusts every host authorized for USB/wireless debugging, as Android does.
 */
class PairingProvider : ContentProvider() {
    override fun onCreate() = true
    private fun authorize() {
        val uid = Binder.getCallingUid()
        if (uid != 2000 && uid != Process.myUid()) throw SecurityException("Pairing requires the authorized Android debugging shell")
    }
    override fun openFile(uri: Uri, mode: String): ParcelFileDescriptor {
        authorize()
        if (uri.path != "/identity" || mode != "r") throw FileNotFoundException("Read-only identity endpoint")
        val bytes = Identity.pairing(context!!).toByteArray(Charsets.UTF_8)
        val pipe = ParcelFileDescriptor.createPipe()
        Thread { ParcelFileDescriptor.AutoCloseOutputStream(pipe[1]).use { it.write(bytes) } }.start()
        return pipe[0]
    }
    override fun getType(uri: Uri): String { authorize(); return "application/json" }
    override fun query(uri: Uri, projection: Array<out String>?, selection: String?, selectionArgs: Array<out String>?, sortOrder: String?): Cursor? { authorize(); return null }
    override fun insert(uri: Uri, values: ContentValues?): Uri? = throw SecurityException("Read-only")
    override fun delete(uri: Uri, selection: String?, selectionArgs: Array<out String>?): Int = throw SecurityException("Read-only")
    override fun update(uri: Uri, values: ContentValues?, selection: String?, selectionArgs: Array<out String>?): Int = throw SecurityException("Read-only")
}
