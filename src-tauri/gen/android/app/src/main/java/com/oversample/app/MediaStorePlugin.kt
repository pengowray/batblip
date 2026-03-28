package com.oversample.app

import android.Manifest
import android.app.Activity
import android.content.ContentUris
import android.content.ContentValues
import android.content.Intent
import android.content.pm.PackageManager
import android.media.MediaScannerConnection
import android.net.Uri
import android.os.Build
import android.os.Environment
import android.provider.MediaStore
import android.util.Log
import androidx.core.app.ActivityCompat
import androidx.core.content.ContextCompat
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import java.io.File

private const val TAG = "MediaStorePlugin"
private const val SUBFOLDER = "Oversample"
private const val WRITE_STORAGE_REQUEST_CODE = 9001

@InvokeArg
data class SaveToSharedArgs(val internalPath: String = "", val filename: String = "", val deleteInternal: Boolean = false)

@InvokeArg
data class SaveWavBytesArgs(val filename: String = "", val data: ByteArray = ByteArray(0))


@InvokeArg
data class ExportFileArgs(val internalPath: String = "", val suggestedName: String = "")

@TauriPlugin
class MediaStorePlugin(private val activity: Activity) : Plugin(activity) {

    private var pendingPermissionInvoke: Invoke? = null
    private var pendingExportInvoke: Invoke? = null
    private var pendingExportBytes: ByteArray? = null
    private var pendingExportMimeType: String? = null

    companion object {
        const val EXPORT_REQUEST_CODE = 9002
    }

    override fun load(webView: android.webkit.WebView) {
        super.load(webView)
        Log.i(TAG, "MediaStorePlugin loaded")
    }

    // ── Save recording to shared storage ────────────────────────────────

    /**
     * Copy a WAV file from internal app storage to shared storage
     * (Recordings/Oversample on API 29+, Music/Oversample on API 24-28).
     * Returns the public path or content URI string.
     */
    @Command
    fun saveToSharedStorage(invoke: Invoke) {
        val args = invoke.parseArgs(SaveToSharedArgs::class.java)
        val internalFile = File(args.internalPath)
        if (!internalFile.exists()) {
            invoke.reject("File not found: ${args.internalPath}")
            return
        }
        val filename = args.filename.ifEmpty { internalFile.name }

        try {
            val resultPath = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
                saveViaMediaStore(internalFile, filename)
            } else {
                if (!hasWritePermission()) {
                    // Request permission and retry
                    pendingPermissionInvoke = invoke
                    requestWritePermission()
                    return
                }
                saveViaDirectFile(internalFile, filename)
            }
            // Delete internal file after successful move
            if (args.deleteInternal) {
                internalFile.delete()
                Log.i(TAG, "Deleted internal copy: ${args.internalPath}")
            }
            val result = JSObject()
            result.put("path", resultPath)
            invoke.resolve(result)
        } catch (e: Exception) {
            Log.e(TAG, "saveToSharedStorage failed", e)
            invoke.reject("Failed to save: ${e.message}")
        }
    }

    /**
     * Save WAV bytes directly to shared storage (Recordings/Oversample).
     * Used when the frontend already has the WAV data and doesn't need
     * to go through Rust internal storage first.
     */
    @Command
    fun saveWavBytes(invoke: Invoke) {
        val args = invoke.parseArgs(SaveWavBytesArgs::class.java)
        if (args.data.isEmpty()) {
            invoke.reject("No data provided")
            return
        }
        val filename = args.filename.ifEmpty { "recording.wav" }

        try {
            val resultPath = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
                saveViaMediaStoreBytes(args.data, filename)
            } else {
                if (!hasWritePermission()) {
                    pendingPermissionInvoke = invoke
                    requestWritePermission()
                    return
                }
                saveViaDirectFileBytes(args.data, filename)
            }
            val result = JSObject()
            result.put("path", resultPath)
            invoke.resolve(result)
        } catch (e: Exception) {
            Log.e(TAG, "saveWavBytes failed", e)
            invoke.reject("Failed to save: ${e.message}")
        }
    }

    // ── Export file via SAF picker ───────────────────────────────────────

    /**
     * Open a system "Save As" dialog for the user to choose where to export a file.
     * Works for any file type (WAV, BATM, etc).
     */
    @Command
    fun exportFile(invoke: Invoke) {
        val args = invoke.parseArgs(ExportFileArgs::class.java)
        val file = File(args.internalPath)
        if (!file.exists()) {
            invoke.reject("File not found: ${args.internalPath}")
            return
        }
        val suggestedName = args.suggestedName.ifEmpty { file.name }
        val mimeType = when {
            suggestedName.endsWith(".wav", ignoreCase = true) -> "audio/wav"
            suggestedName.endsWith(".flac", ignoreCase = true) -> "audio/flac"
            suggestedName.endsWith(".mp3", ignoreCase = true) -> "audio/mpeg"
            suggestedName.endsWith(".ogg", ignoreCase = true) -> "audio/ogg"
            suggestedName.endsWith(".batm", ignoreCase = true) -> "application/octet-stream"
            suggestedName.endsWith(".yaml", ignoreCase = true) -> "text/yaml"
            suggestedName.endsWith(".yml", ignoreCase = true) -> "text/yaml"
            else -> "application/octet-stream"
        }

        pendingExportInvoke = invoke
        pendingExportBytes = file.readBytes()
        pendingExportMimeType = mimeType

        val intent = Intent(Intent.ACTION_CREATE_DOCUMENT).apply {
            addCategory(Intent.CATEGORY_OPENABLE)
            type = mimeType
            putExtra(Intent.EXTRA_TITLE, suggestedName)
        }
        activity.startActivityForResult(intent, EXPORT_REQUEST_CODE)
    }

    // ── Activity result handling (SAF) ──────────────────────────────────

    fun handleActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        if (requestCode != EXPORT_REQUEST_CODE) return

        val invoke = pendingExportInvoke ?: return
        pendingExportInvoke = null
        val bytes = pendingExportBytes
        pendingExportBytes = null
        pendingExportMimeType = null

        if (resultCode != Activity.RESULT_OK || data?.data == null) {
            val result = JSObject()
            result.put("cancelled", true)
            invoke.resolve(result)
            return
        }

        val uri = data.data!!
        try {
            activity.contentResolver.openOutputStream(uri)?.use { out ->
                out.write(bytes)
            }
            val result = JSObject()
            result.put("cancelled", false)
            result.put("uri", uri.toString())
            invoke.resolve(result)
        } catch (e: Exception) {
            Log.e(TAG, "SAF export failed", e)
            invoke.reject("Export failed: ${e.message}")
        }
    }

    // ── Permission handling ─────────────────────────────────────────────

    fun handlePermissionResult(requestCode: Int, grantResults: IntArray) {
        if (requestCode != WRITE_STORAGE_REQUEST_CODE) return
        val invoke = pendingPermissionInvoke ?: return
        pendingPermissionInvoke = null

        if (grantResults.isNotEmpty() && grantResults[0] == PackageManager.PERMISSION_GRANTED) {
            // Retry the original command — re-invoke via the plugin command
            // For simplicity, just tell the frontend to retry
            val result = JSObject()
            result.put("permissionGranted", true)
            result.put("retry", true)
            invoke.resolve(result)
        } else {
            invoke.reject("Storage permission denied")
        }
    }

    // ── Internal helpers ────────────────────────────────────────────────

    /** API 29+: Save via MediaStore to Recordings/Oversample */
    private fun saveViaMediaStore(sourceFile: File, displayName: String): String {
        val resolver = activity.contentResolver

        val values = ContentValues().apply {
            put(MediaStore.Audio.Media.DISPLAY_NAME, displayName)
            put(MediaStore.Audio.Media.MIME_TYPE, "audio/wav")
            put(MediaStore.Audio.Media.RELATIVE_PATH, "Recordings/$SUBFOLDER")
            put(MediaStore.Audio.Media.IS_PENDING, 1)
        }

        val collection = MediaStore.Audio.Media.getContentUri(MediaStore.VOLUME_EXTERNAL_PRIMARY)
        val uri = resolver.insert(collection, values)
            ?: throw Exception("Failed to create MediaStore entry for $displayName")

        resolver.openOutputStream(uri)?.use { outputStream ->
            sourceFile.inputStream().use { inputStream ->
                inputStream.copyTo(outputStream, bufferSize = 8192)
            }
        } ?: throw Exception("Failed to open output stream for $displayName")

        // Mark as complete
        values.clear()
        values.put(MediaStore.Audio.Media.IS_PENDING, 0)
        resolver.update(uri, values, null, null)

        Log.i(TAG, "Saved to MediaStore: $displayName -> $uri")
        return uri.toString()
    }

    /** API 24-28: Save via direct file I/O to shared storage */
    @Suppress("DEPRECATION")
    private fun saveViaDirectFile(sourceFile: File, displayName: String): String {
        // Use Recordings/ directory (create it if needed, works fine on pre-Q)
        val baseDir = File(Environment.getExternalStorageDirectory(), "Recordings")
        val appDir = File(baseDir, SUBFOLDER)
        appDir.mkdirs()

        val destFile = File(appDir, displayName)
        sourceFile.inputStream().use { input ->
            destFile.outputStream().use { output ->
                input.copyTo(output, bufferSize = 8192)
            }
        }

        // Notify MediaStore so it appears in media apps / file managers
        MediaScannerConnection.scanFile(
            activity,
            arrayOf(destFile.absolutePath),
            arrayOf("audio/wav"),
            null
        )

        Log.i(TAG, "Saved to file: ${destFile.absolutePath}")
        return destFile.absolutePath
    }

    /** API 29+: Save raw bytes via MediaStore to Recordings/Oversample */
    private fun saveViaMediaStoreBytes(data: ByteArray, displayName: String): String {
        val resolver = activity.contentResolver

        val values = ContentValues().apply {
            put(MediaStore.Audio.Media.DISPLAY_NAME, displayName)
            put(MediaStore.Audio.Media.MIME_TYPE, "audio/wav")
            put(MediaStore.Audio.Media.RELATIVE_PATH, "Recordings/$SUBFOLDER")
            put(MediaStore.Audio.Media.IS_PENDING, 1)
        }

        val collection = MediaStore.Audio.Media.getContentUri(MediaStore.VOLUME_EXTERNAL_PRIMARY)
        val uri = resolver.insert(collection, values)
            ?: throw Exception("Failed to create MediaStore entry for $displayName")

        resolver.openOutputStream(uri)?.use { outputStream ->
            outputStream.write(data)
        } ?: throw Exception("Failed to open output stream for $displayName")

        values.clear()
        values.put(MediaStore.Audio.Media.IS_PENDING, 0)
        resolver.update(uri, values, null, null)

        Log.i(TAG, "Saved bytes to MediaStore: $displayName -> $uri")
        return uri.toString()
    }

    /** API 24-28: Save raw bytes via direct file I/O to shared storage */
    @Suppress("DEPRECATION")
    private fun saveViaDirectFileBytes(data: ByteArray, displayName: String): String {
        val baseDir = File(Environment.getExternalStorageDirectory(), "Recordings")
        val appDir = File(baseDir, SUBFOLDER)
        appDir.mkdirs()

        val destFile = File(appDir, displayName)
        destFile.writeBytes(data)

        MediaScannerConnection.scanFile(
            activity,
            arrayOf(destFile.absolutePath),
            arrayOf("audio/wav"),
            null
        )

        Log.i(TAG, "Saved bytes to file: ${destFile.absolutePath}")
        return destFile.absolutePath
    }

    private fun hasWritePermission(): Boolean {
        return ContextCompat.checkSelfPermission(
            activity,
            Manifest.permission.WRITE_EXTERNAL_STORAGE
        ) == PackageManager.PERMISSION_GRANTED
    }

    private fun requestWritePermission() {
        ActivityCompat.requestPermissions(
            activity,
            arrayOf(Manifest.permission.WRITE_EXTERNAL_STORAGE),
            WRITE_STORAGE_REQUEST_CODE
        )
    }
}
