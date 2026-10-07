package com.saysky.agecivmodtool

import android.app.Activity
import android.content.Intent
import android.net.Uri
import android.os.Build
import android.os.Environment
import android.provider.DocumentsContract
import android.provider.Settings
import androidx.activity.result.ActivityResult
import app.tauri.annotation.ActivityCallback
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import java.io.File

@InvokeArg
class ResolveFolderPathArgs {
    lateinit var folderId: String
}

/**
 * 「所有文件访问」权限与 SAF 真实路径解析插件。
 *
 * 文件选择/读写/目录/复制移动已全部迁移到 tauri-plugin-android-fs，本插件仅保留：
 * - 权限申请与查询（Android 11+ 需 MANAGE_EXTERNAL_STORAGE 才能走真实路径模式）；
 * - resolveFolderPath：把 SAF 目录树解析为真实文件系统路径，让 Rust 侧绕过
 *   ContentProvider 用 std::fs 直接多线程读写（解压、事件文件、图标批量载入）。
 */
@TauriPlugin
class AllFilesAccessPlugin(private val activity: Activity) : Plugin(activity) {

    private fun hasAllFilesAccess(): Boolean {
        return Build.VERSION.SDK_INT < Build.VERSION_CODES.R || Environment.isExternalStorageManager()
    }

    private fun accessStatus(): JSObject {
        return JSObject().apply { put("granted", hasAllFilesAccess()) }
    }

    @Command
    fun hasAllFilesAccess(invoke: Invoke) {
        invoke.resolve(accessStatus())
    }

    @Command
    fun requestAllFilesAccess(invoke: Invoke) {
        if (hasAllFilesAccess()) {
            invoke.resolve(accessStatus())
            return
        }

        val appSettingsIntent = Intent(Settings.ACTION_MANAGE_APP_ALL_FILES_ACCESS_PERMISSION).apply {
            data = Uri.parse("package:${activity.packageName}")
        }
        try {
            startActivityForResult(invoke, appSettingsIntent, "onSettingsResult")
        } catch (_: Exception) {
            try {
                startActivityForResult(
                    invoke,
                    Intent(Settings.ACTION_MANAGE_ALL_FILES_ACCESS_PERMISSION),
                    "onSettingsResult",
                )
            } catch (error: Exception) {
                invoke.reject(error.message ?: "无法打开所有文件访问设置")
            }
        }
    }

    @ActivityCallback
    fun onSettingsResult(invoke: Invoke, result: ActivityResult) {
        invoke.resolve(accessStatus())
    }

    /**
     * 解析 SAF 目录树的真实文件系统路径（folderId 即目录树 URI 字符串）。
     * 仅在已授权且目录位于共享存储（primary/存储卡）时返回路径。
     */
    @Command
    fun resolveFolderPath(invoke: Invoke) {
        val args = invoke.parseArgs(ResolveFolderPathArgs::class.java)
        Thread {
            try {
                val path = resolveRealFolderPath(args.folderId)
                invoke.resolve(JSObject().apply { put("path", path) })
            } catch (error: Throwable) {
                invoke.reject(error.message ?: "解析目录真实路径失败")
            }
        }.start()
    }

    private fun resolveRealFolderPath(folderId: String): String? {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.R || !Environment.isExternalStorageManager()) {
            return null
        }
        val documentId = try {
            DocumentsContract.getTreeDocumentId(Uri.parse(folderId))
        } catch (_: Throwable) {
            return null
        }
        val separator = documentId.indexOf(':')
        val volumeId = if (separator >= 0) documentId.substring(0, separator) else documentId
        val relativePath = if (separator >= 0) documentId.substring(separator + 1) else ""
        val root = when {
            volumeId.equals("primary", ignoreCase = true) -> Environment.getExternalStorageDirectory()
            else -> findVolumeRoot(volumeId)
        } ?: return null
        val candidate = if (relativePath.isEmpty()) root else File(root, relativePath)
        return if (candidate.isDirectory && candidate.canRead()) candidate.absolutePath else null
    }

    private fun findVolumeRoot(volumeId: String): File? {
        return try {
            val manager = activity.getSystemService(android.os.storage.StorageManager::class.java)
            manager.storageVolumes.firstOrNull { it.uuid?.equals(volumeId, ignoreCase = true) == true }?.directory
        } catch (_: Throwable) {
            null
        }
    }
}
