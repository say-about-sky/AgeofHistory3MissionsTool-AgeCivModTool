package com.saysky.agecivmodtool

import android.app.Activity
import android.content.Intent
import android.net.Uri
import android.os.Build
import android.os.Environment
import android.os.SystemClock
import android.provider.DocumentsContract
import android.provider.Settings
import android.util.Log
import androidx.activity.result.ActivityResult
import androidx.documentfile.provider.DocumentFile
import app.tauri.annotation.ActivityCallback
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSArray
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import java.io.File

/** 目录子项缓存有效期：60 秒内的重复查询直接复用内存列表，避免重复游标扫描。 */
private const val CHILDREN_CACHE_TTL_MS = 60_000L
private const val CHILDREN_CACHE_MAX_ENTRIES = 64

/** adb logcat -s AgeCivPerf:D 可查看关键路径耗时。 */
private const val PERF_TAG = "AgeCivPerf"

@InvokeArg
class ListDirArgs {
    lateinit var folderId: String
    var path: String? = null
}

@InvokeArg
class ReadFilesInDirArgs {
    lateinit var folderId: String
    lateinit var dirPath: String
    var names: Array<String> = emptyArray()
}

@InvokeArg
class ReadTextInDirArgs {
    lateinit var folderId: String
    lateinit var dirPath: String
    lateinit var fileName: String
}

@InvokeArg
class WriteTextInDirArgs {
    lateinit var folderId: String
    lateinit var dirPath: String
    lateinit var fileName: String
    lateinit var contents: String
}

@InvokeArg
class ResolveFolderPathArgs {
    lateinit var folderId: String
}

@TauriPlugin
class AllFilesAccessPlugin(private val activity: Activity) : Plugin(activity) {
    /** 目录子项内存缓存：键 "treeUri|relativePath"；仅加速只读查询，写操作会失效对应项。 */
    private val childrenCache = HashMap<String, CachedChildren>()

    private data class CachedChildren(val children: MutableList<ChildEntry>, val time: Long)

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
     * 解析工作区 SAF 目录对应的真实文件系统路径。
     * 仅在已授予"所有文件访问"且目录位于共享存储（primary/存储卡）时返回路径，
     * Rust 侧随后可绕过 ContentProvider 直接多线程读写，显著提速。
     */
    @Command
    fun resolveFolderPath(invoke: Invoke) {
        val args = invoke.parseArgs(ResolveFolderPathArgs::class.java)
        Thread {
            try {
                val path = resolveRealFolderPath(args.folderId)
                Log.d(PERF_TAG, "resolveFolderPath id=${args.folderId} path=$path")
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
        val treeUri = try {
            folderTreeUri(folderId)
        } catch (_: Throwable) {
            return null
        }
        val documentId = try {
            DocumentsContract.getTreeDocumentId(treeUri)
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

    /** 读取工作区目录：单次子项游标查询列出一层目录，避免逐文件属性查询导致的卡顿。 */
    @Command
    fun listDir(invoke: Invoke) {
        val args = invoke.parseArgs(ListDirArgs::class.java)
        Thread {
            try {
                val entries = listDirEntries(args.folderId, args.path ?: "")
                invoke.resolve(JSObject().apply { put("entries", entries) })
            } catch (error: Throwable) {
                invoke.reject(error.message ?: "读取目录失败")
            }
        }.start()
    }

    /** 批量读取目录内图标：单次列目录、按文件名匹配后直读，返回 name + dataUrl（缺失项跳过）。 */
    @Command
    fun readFilesInDir(invoke: Invoke) {
        val args = invoke.parseArgs(ReadFilesInDirArgs::class.java)
        Thread {
            try {
                val files = readFilesInDirEntries(args.folderId, args.dirPath, args.names)
                invoke.resolve(JSObject().apply { put("files", files) })
            } catch (error: Throwable) {
                invoke.reject(error.message ?: "读取图标失败")
            }
        }.start()
    }

    /** 读取目录内单个文本文件（快速路径：单次列目录 + 直读，避免逐名查询）。 */
    @Command
    fun readTextFileInDir(invoke: Invoke) {
        val args = invoke.parseArgs(ReadTextInDirArgs::class.java)
        Thread {
            try {
                val text = readTextInDir(args.folderId, args.dirPath, args.fileName)
                invoke.resolve(JSObject().apply { put("contents", text) })
            } catch (error: Throwable) {
                invoke.reject(error.message ?: "读取文件失败")
            }
        }.start()
    }

    /** 写入（必要时创建）目录内单个文本文件。 */
    @Command
    fun writeTextFileInDir(invoke: Invoke) {
        val args = invoke.parseArgs(WriteTextInDirArgs::class.java)
        Thread {
            try {
                writeTextInDir(args.folderId, args.dirPath, args.fileName, args.contents)
                invoke.resolve()
            } catch (error: Throwable) {
                invoke.reject(error.message ?: "写入文件失败")
            }
        }.start()
    }

    /** 在目录中定位单个子项：优先内存缓存，未命中则流式扫描（只对命中行构建 URI，找到即停）。 */
    private fun findChildUri(treeUri: Uri, dirPath: String, fileName: String): Uri? {
        val started = SystemClock.elapsedRealtime()
        var fromCache = true
        var scanned = 0
        var child = findCachedChild(treeUri, dirPath, fileName)
        if (child == null) {
            fromCache = false
            val children = try {
                listChildrenFast(treeUri, dirPath)
            } catch (_: FastListUnsupportedException) {
                listChildrenSlow(treeUri, dirPath).also { storeChildren(treeUri, dirPath, it.toMutableList()) }
            }
            scanned = children.size
            child = matchChild(children, fileName)
        }
        val uri = child?.let { DocumentsContract.buildDocumentUriUsingTree(treeUri, it.documentId) }
        Log.d(
            PERF_TAG,
            "findChild $dirPath/$fileName cache=$fromCache scanned=$scanned found=${uri != null} took=${SystemClock.elapsedRealtime() - started}ms",
        )
        return uri
    }

    private fun matchChild(children: List<ChildEntry>, fileName: String): ChildEntry? {
        children.firstOrNull { !it.isDir && it.name == fileName }?.let { return it }
        val lower = fileName.lowercase()
        return children.firstOrNull { !it.isDir && it.name.lowercase() == lower }
    }

    private fun readTextInDir(folderId: String, dirPath: String, fileName: String): String {
        val treeUri = folderTreeUri(folderId)
        val started = SystemClock.elapsedRealtime()
        var lastError = "文件不存在：$fileName"
        for (attempt in 0..1) {
            val uri = findChildUri(treeUri, dirPath, fileName)
            if (uri == null) {
                // 缓存或上次扫描可能过期：失效后重扫一次。
                invalidateChildren(treeUri, dirPath)
                if (attempt == 1) {
                    throw IllegalStateException("文件不存在：$fileName")
                }
                continue
            }
            try {
                val bytes = activity.contentResolver.openInputStream(uri)?.use { it.readBytes() }
                    ?: throw IllegalStateException("无法读取文件：$fileName")
                Log.d(PERF_TAG, "readText $fileName took=${SystemClock.elapsedRealtime() - started}ms")
                return String(bytes, Charsets.UTF_8)
            } catch (error: Throwable) {
                lastError = error.message ?: "无法读取文件：$fileName"
                invalidateChildren(treeUri, dirPath)
            }
        }
        throw IllegalStateException(lastError)
    }

    private fun writeTextInDir(folderId: String, dirPath: String, fileName: String, contents: String) {
        val treeUri = folderTreeUri(folderId)
        val bytes = contents.toByteArray(Charsets.UTF_8)
        var lastError = "无法写入文件：$fileName"
        for (attempt in 0..1) {
            val existing = findChildUri(treeUri, dirPath, fileName)
            if (existing != null) {
                try {
                    activity.contentResolver.openOutputStream(existing, "wt")?.use { it.write(bytes) }
                        ?: throw IllegalStateException("无法写入文件：$fileName")
                    return
                } catch (error: Throwable) {
                    lastError = error.message ?: lastError
                    invalidateChildren(treeUri, dirPath)
                    continue
                }
            }
            try {
                val dirUri = try {
                    resolveDirUriFast(treeUri, dirPath)
                } catch (_: FastListUnsupportedException) {
                    resolveDirUriSlow(treeUri, dirPath)
                }
                val created = DocumentsContract.createDocument(
                    activity.contentResolver,
                    dirUri,
                    "text/plain",
                    fileName,
                ) ?: throw IllegalStateException("无法创建文件：$fileName")
                activity.contentResolver.openOutputStream(created, "wt")?.use { it.write(bytes) }
                    ?: throw IllegalStateException("无法写入文件：$fileName")
                rememberCreatedChild(
                    treeUri,
                    dirPath,
                    ChildEntry(DocumentsContract.getDocumentId(created), fileName, false),
                )
                return
            } catch (error: Throwable) {
                lastError = error.message ?: lastError
                invalidateChildren(treeUri, dirPath)
            }
        }
        throw IllegalStateException(lastError)
    }

    private fun resolveDirUriSlow(treeUri: Uri, relativePath: String): Uri {
        var current = DocumentFile.fromTreeUri(activity, treeUri)
            ?: throw IllegalStateException("工作区授权已失效，请重新选择目录")
        for (segment in pathSegments(relativePath)) {
            current = current.findFile(segment)
                ?: throw IllegalStateException("目录不存在：$relativePath")
        }
        return current.uri
    }

    private class FastListUnsupportedException(message: String) : Exception(message)

    private class ChildEntry(val documentId: String, val name: String, val isDir: Boolean)

    private fun folderTreeUri(folderId: String): Uri {
        val prefs = activity.getSharedPreferences("scoped_storage", Activity.MODE_PRIVATE)
        val uriValue = prefs.getString("folder:$folderId:uri", null)
            ?: throw IllegalStateException("工作区授权已失效，请重新选择目录")
        return Uri.parse(uriValue)
    }

    private fun cacheKey(treeUri: Uri, relativePath: String): String = "$treeUri|$relativePath"

    private fun cachedChildren(treeUri: Uri, relativePath: String): MutableList<ChildEntry>? {
        synchronized(childrenCache) {
            val key = cacheKey(treeUri, relativePath)
            val entry = childrenCache[key] ?: return null
            if (SystemClock.elapsedRealtime() - entry.time > CHILDREN_CACHE_TTL_MS) {
                childrenCache.remove(key)
                return null
            }
            return entry.children
        }
    }

    private fun storeChildren(treeUri: Uri, relativePath: String, children: MutableList<ChildEntry>) {
        synchronized(childrenCache) {
            if (childrenCache.size >= CHILDREN_CACHE_MAX_ENTRIES) {
                childrenCache.clear()
            }
            childrenCache[cacheKey(treeUri, relativePath)] =
                CachedChildren(children, SystemClock.elapsedRealtime())
        }
    }

    private fun invalidateChildren(treeUri: Uri, relativePath: String) {
        synchronized(childrenCache) {
            childrenCache.remove(cacheKey(treeUri, relativePath))
        }
    }

    /** 向已有缓存追加新建子项；缓存缺失时保持缺失（下次查询自然重扫），避免留下不完整列表。 */
    private fun rememberCreatedChild(treeUri: Uri, relativePath: String, child: ChildEntry) {
        synchronized(childrenCache) {
            childrenCache[cacheKey(treeUri, relativePath)]?.children?.add(child)
        }
    }

    private fun findCachedChild(treeUri: Uri, relativePath: String, fileName: String): ChildEntry? {
        val children = cachedChildren(treeUri, relativePath) ?: return null
        return matchChild(children, fileName)
    }

    private fun listDirEntries(folderId: String, relativePath: String): JSArray {
        val treeUri = folderTreeUri(folderId)
        return try {
            listDirEntriesFast(treeUri, relativePath)
        } catch (_: FastListUnsupportedException) {
            listDirEntriesSlow(treeUri, relativePath)
        }
    }

    /** 快速路径：每层目录仅 1 次 ContentProvider 子项查询。 */
    private fun listDirEntriesFast(treeUri: Uri, relativePath: String): JSArray {
        val results = JSArray()
        for (child in listChildrenFast(treeUri, relativePath)) {
            results.put(entryObject(child.name, joinPath(relativePath, child.name), child.isDir))
        }
        return results
    }

    /** 快速解析目录路径为 document URI（每层 1 次子项查询）。 */
    private fun resolveDirUriFast(treeUri: Uri, relativePath: String): Uri {
        var dirUri = DocumentsContract.buildDocumentUriUsingTree(
            treeUri,
            DocumentsContract.getTreeDocumentId(treeUri),
        )
        for (segment in pathSegments(relativePath)) {
            val child = queryChildren(treeUri, dirUri).firstOrNull { it.name == segment }
                ?: throw IllegalStateException("目录不存在：$relativePath")
            if (!child.isDir) {
                throw IllegalStateException("路径不是目录：$relativePath")
            }
            dirUri = DocumentsContract.buildDocumentUriUsingTree(treeUri, child.documentId)
        }
        return dirUri
    }

    /** 列出目录子项（带 60 秒内存缓存）：命中时零 ContentProvider 查询。 */
    private fun listChildrenFast(treeUri: Uri, relativePath: String): List<ChildEntry> {
        cachedChildren(treeUri, relativePath)?.let { return it }
        val children = queryChildren(treeUri, resolveDirUriFast(treeUri, relativePath)).toMutableList()
        storeChildren(treeUri, relativePath, children)
        return children
    }

    private fun listChildrenSlow(treeUri: Uri, relativePath: String): List<ChildEntry> {
        var current = DocumentFile.fromTreeUri(activity, treeUri)
            ?: throw IllegalStateException("工作区授权已失效，请重新选择目录")
        for (segment in pathSegments(relativePath)) {
            current = current.findFile(segment)
                ?: throw IllegalStateException("目录不存在：$relativePath")
        }
        val out = ArrayList<ChildEntry>()
        for (file in current.listFiles()) {
            val name = file.name ?: continue
            val documentId = try {
                DocumentsContract.getDocumentId(file.uri)
            } catch (_: Throwable) {
                continue
            }
            out.add(ChildEntry(documentId, name, file.isDirectory))
        }
        return out
    }

    /** 批量读取目录内图标：按 "{name}.png" 大小写不敏感流式匹配（只对命中行构建 URI，全部命中即停）。 */
    private fun readFilesInDirEntries(
        folderId: String,
        dirPath: String,
        names: Array<String>,
    ): JSArray {
        val started = SystemClock.elapsedRealtime()
        val treeUri = folderTreeUri(folderId)
        val wanted = LinkedHashMap<String, String>()
        for (name in names) {
            if (name.isBlank() || name.contains('/') || name.contains('\\')) {
                continue
            }
            val key = "${name.lowercase()}.png"
            if (!wanted.containsKey(key)) {
                wanted[key] = name
            }
        }
        val matched = HashMap<String, String>()
        if (wanted.isNotEmpty()) {
            cachedChildren(treeUri, dirPath)?.let { fillMatched(it, wanted, matched) }
            if (matched.size < wanted.size) {
                invalidateChildren(treeUri, dirPath)
                val children = try {
                    listChildrenFast(treeUri, dirPath)
                } catch (_: FastListUnsupportedException) {
                    listChildrenSlow(treeUri, dirPath).also { storeChildren(treeUri, dirPath, it.toMutableList()) }
                }
                fillMatched(children, wanted, matched)
            }
        }

        val files = JSArray()
        for ((key, originalName) in wanted) {
            val documentId = matched[key] ?: continue
            val uri = DocumentsContract.buildDocumentUriUsingTree(treeUri, documentId)
            val bytes = try {
                activity.contentResolver.openInputStream(uri)?.use { it.readBytes() }
            } catch (_: Throwable) {
                null
            } ?: continue
            val encoded = android.util.Base64.encodeToString(bytes, android.util.Base64.NO_WRAP)
            files.put(JSObject().apply {
                put("name", originalName)
                put("dataUrl", "data:image/png;base64,$encoded")
            })
        }
        Log.d(
            PERF_TAG,
            "readFilesInDir dir=$dirPath requested=${wanted.size} matched=${matched.size} took=${SystemClock.elapsedRealtime() - started}ms",
        )
        return files
    }

    private fun fillMatched(
        children: List<ChildEntry>,
        wanted: Map<String, String>,
        matched: MutableMap<String, String>,
    ) {
        for (child in children) {
            if (child.isDir) {
                continue
            }
            val key = child.name.lowercase()
            if (wanted.containsKey(key) && !matched.containsKey(key)) {
                matched[key] = child.documentId
                if (matched.size >= wanted.size) {
                    return
                }
            }
        }
    }

    /** 兜底路径：个别不支持子项游标的目录服务，退回 DocumentFile 逐项读取（较慢但能工作）。 */
    private fun listDirEntriesSlow(treeUri: Uri, relativePath: String): JSArray {
        var current = DocumentFile.fromTreeUri(activity, treeUri)
            ?: throw IllegalStateException("工作区授权已失效，请重新选择目录")
        for (segment in pathSegments(relativePath)) {
            current = current.findFile(segment)
                ?: throw IllegalStateException("目录不存在：$relativePath")
        }

        val results = JSArray()
        for (file in current.listFiles()) {
            val name = file.name ?: continue
            results.put(entryObject(name, joinPath(relativePath, name), file.isDirectory))
        }
        return results
    }

    private fun queryChildren(treeUri: Uri, dirUri: Uri): List<ChildEntry> {
        val out = ArrayList<ChildEntry>()
        forEachChild(treeUri, dirUri) { documentId, name, isDir ->
            out.add(ChildEntry(documentId, name, isDir))
            true
        }
        return out
    }

    /** 流式遍历子项游标；visit 返回 false 时立即停止（不再读取后续行），用于按名查找的提前退出。 */
    private inline fun forEachChild(
        treeUri: Uri,
        dirUri: Uri,
        visit: (documentId: String, name: String, isDir: Boolean) -> Boolean,
    ) {
        val childrenUri = DocumentsContract.buildChildDocumentsUriUsingTree(
            dirUri,
            DocumentsContract.getDocumentId(dirUri),
        )
        val projection = arrayOf(
            DocumentsContract.Document.COLUMN_DOCUMENT_ID,
            DocumentsContract.Document.COLUMN_DISPLAY_NAME,
            DocumentsContract.Document.COLUMN_MIME_TYPE,
        )
        val cursor = activity.contentResolver.query(childrenUri, projection, null, null, null)
            ?: throw FastListUnsupportedException("无法读取目录列表")
        cursor.use { c ->
            val idIndex = c.getColumnIndex(DocumentsContract.Document.COLUMN_DOCUMENT_ID)
            val nameIndex = c.getColumnIndex(DocumentsContract.Document.COLUMN_DISPLAY_NAME)
            val mimeIndex = c.getColumnIndex(DocumentsContract.Document.COLUMN_MIME_TYPE)
            if (idIndex < 0 || nameIndex < 0 || mimeIndex < 0) {
                throw FastListUnsupportedException("目录服务不支持列投影")
            }
            while (c.moveToNext()) {
                val documentId = c.getString(idIndex) ?: continue
                val name = c.getString(nameIndex) ?: ""
                val isDir = c.getString(mimeIndex) == DocumentsContract.Document.MIME_TYPE_DIR
                if (!visit(documentId, name, isDir)) {
                    return
                }
            }
        }
    }

    private fun pathSegments(relativePath: String): List<String> =
        relativePath.split('/').filter { it.isNotEmpty() }

    private fun joinPath(base: String, name: String): String =
        if (base.isEmpty()) name else "$base/$name"

    private fun entryObject(name: String, relativePath: String, isDir: Boolean): JSObject =
        JSObject().apply {
            put("name", name)
            put("path", relativePath)
            put("isDir", isDir)
        }
}