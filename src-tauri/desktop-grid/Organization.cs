using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Security.Cryptography;
using System.Text;

namespace SoulDesk.DesktopGrid
{
    public sealed class OrganizedItem
    {
        public string Id, Batch, Tab, Title, Source, Destination, Fingerprint, Status, Error;
        public bool Directory, Mapped, LayoutSynced;
        public DateTime Timestamp;
    }

    // 先提交可恢复的清单，再原子改名。每项单独记账，失败时从原路径重新确认。
    internal sealed class GridOrganization
    {
        private readonly string root;
        private readonly string path;
        private readonly object gate = new object();
        private const int HistoryLimit = 600;

        internal GridOrganization(string root)
        {
            this.root = root;
            path = Path.Combine(root, "organization-history.json");
        }

        internal List<OrganizedItem> Read()
        {
            return File.Exists(path) ? LocalJson.Read<List<OrganizedItem>>(path) : new List<OrganizedItem>();
        }

        internal OrganizedItem[] Plan(IEnumerable<KeyValuePair<DesktopItem, GridTab>> changes)
        {
            string batch = GridState.NewId();
            return changes.Select(change =>
            {
                var tab = change.Value;
                string folder = tab.Folder ?? Path.Combine(root, "organized", tab.Id);
                return new OrganizedItem {
                    Id = GridState.NewId(), Batch = batch, Tab = tab.Id, Title = tab.Name,
                    Source = change.Key.Id, Destination = Path.Combine(folder, Path.GetFileName(change.Key.Id)),
                    Mapped = tab.Folder != null, Timestamp = DateTime.UtcNow, Status = "new"
                };
            }).ToArray();
        }

        internal void Move(OrganizedItem[] plan)
        {
            lock (gate)
            {
                var history = Read();
                if (history.Count + plan.Length > HistoryLimit)
                    throw new InvalidOperationException("整理历史已满，请先撤回或清理已结束的记录。");
                foreach (var item in plan)
                {
                    try
                    {
                        ValidateMove(item.Source, item.Destination);
                        string targetFolder = Path.GetDirectoryName(item.Destination);
                        if (!item.Mapped) Directory.CreateDirectory(targetFolder);
                        CheckPath(targetFolder);
                        item.Directory = System.IO.Directory.Exists(item.Source);
                        item.Fingerprint = Fingerprint(item.Source, item.Directory);
                    }
                    catch (Exception error)
                    {
                        if (!IsFileError(error)) throw;
                        item.Status = "failed";
                        item.Error = error.Message;
                        history.Insert(0, item);
                        Save(history);
                        continue;
                    }
                    item.Status = "moving";
                    history.Insert(0, item);
                    Save(history);
                    Transfer(item, item.Source, item.Destination, "moved", "failed");
                    Save(history);
                }
            }
        }

        internal OrganizedItem Restore(string id)
        {
            lock (gate)
            {
                var history = Read();
                var item = history.SingleOrDefault(entry => entry.Id == id && entry.Status == "moved");
                if (item == null) throw new InvalidOperationException("整理记录已变更。");
                ValidateMove(item.Destination, item.Source);
                CheckPath(Path.GetDirectoryName(item.Source));
                if (Fingerprint(item.Destination, item.Directory) != item.Fingerprint)
                    throw new InvalidOperationException("搬移后的内容已修改，请在资源管理器手动处理。");
                item.Status = "restoring";
                item.LayoutSynced = false;
                item.Error = null;
                Save(history);
                Transfer(item, item.Destination, item.Source, "restored", "moved");
                Save(history);
                return item;
            }
        }

        internal void Recover()
        {
            lock (gate)
            {
                var history = Read();
                bool changed = false;
                foreach (var item in history.Where(entry => entry.Status == "moving" || entry.Status == "restoring"))
                {
                    try
                    {
                        bool source = Exists(item.Source), destination = Exists(item.Destination);
                        if (source == destination) continue;
                        string existing = source ? item.Source : item.Destination;
                        CheckPath(existing);
                        if (Fingerprint(existing, item.Directory) != item.Fingerprint) continue;
                        item.Status = destination ? "moved" : item.Status == "restoring" ? "restored" : "failed";
                        changed = true;
                    }
                    catch (Exception error) { if (!IsFileError(error)) throw; }
                }
                if (changed) Save(history);
            }
        }

        internal void ClearFinished()
        {
            lock (gate)
            {
                var history = Read();
                history.RemoveAll(item => item.Status == "failed" || item.Status == "restored");
                Save(history);
            }
        }

        internal void MarkLayoutSynced(IEnumerable<OrganizedItem> items)
        {
            var ids = new HashSet<string>(items.Select(item => item.Id), StringComparer.Ordinal);
            if (ids.Count == 0) return;
            lock (gate)
            {
                var history = Read();
                foreach (var item in history.Where(item => ids.Contains(item.Id))) item.LayoutSynced = true;
                Save(history);
            }
        }

        private void Save(List<OrganizedItem> history) { LocalJson.Write(path, history); }

        private static void Transfer(OrganizedItem item, string origin, string target, string success, string failure)
        {
            try
            {
                if (item.Directory) System.IO.Directory.Move(origin, target);
                else File.Move(origin, target);
                item.Status = success;
            }
            catch (Exception error)
            {
                if (!IsFileError(error)) throw;
                item.Status = failure;
                item.Error = error.Message;
            }
        }

        private static bool IsFileError(Exception error)
        {
            return error is IOException || error is UnauthorizedAccessException || error is InvalidOperationException ||
                error is ArgumentException || error is NotSupportedException;
        }

        private static bool Exists(string path) { return File.Exists(path) || System.IO.Directory.Exists(path); }

        private static void ValidateMove(string source, string target)
        {
            if (!References.IsLocalPath(source) || !References.IsLocalPath(target) ||
                !References.Equal(Path.GetFullPath(source), source) || !References.Equal(Path.GetFullPath(target), target) ||
                !References.Equal(Path.GetPathRoot(source), Path.GetPathRoot(target)))
                throw new InvalidOperationException("原文件搬移仅支持同一磁盘的本地路径。");
            if (!Exists(source) || Exists(target) ||
                target.StartsWith(source.TrimEnd('\\') + "\\", StringComparison.OrdinalIgnoreCase))
                throw new InvalidOperationException("源文件已变更，目标已存在，或目标在源目录内。");
            CheckPath(source);
        }

        private static void CheckPath(string path)
        {
            for (string current = path; !string.IsNullOrEmpty(current); current = Path.GetDirectoryName(current))
                if ((File.GetAttributes(current) & FileAttributes.ReparsePoint) != 0)
                    throw new InvalidOperationException("不能搬移重解析点或其下的项目。");
        }

        private static string Fingerprint(string path, bool directory)
        {
            if (directory != System.IO.Directory.Exists(path))
                throw new InvalidOperationException("源项目类型已变更。");
            using (var algorithm = SHA256.Create())
            using (var input = new CryptoStream(Stream.Null, algorithm, CryptoStreamMode.Write))
            {
                var paths = new Stack<string>();
                paths.Push(path);
                while (paths.Count != 0)
                {
                    string current = paths.Pop();
                    var attributes = File.GetAttributes(current);
                    if ((attributes & FileAttributes.ReparsePoint) != 0)
                        throw new InvalidOperationException("不能搬移包含重解析点的目录。");
                    string relative = current.Substring(path.Length);
                    bool folder = (attributes & FileAttributes.Directory) != 0;
                    byte[] header = Encoding.UTF8.GetBytes((folder ? "D" : "F") + relative + "\0" +
                        (folder ? "" : new FileInfo(current).Length.ToString()) + "\0");
                    input.Write(header, 0, header.Length);
                    if (!folder)
                        using (var file = new FileStream(current, FileMode.Open, FileAccess.Read, FileShare.Read))
                            file.CopyTo(input);
                    else
                        foreach (string child in System.IO.Directory.EnumerateFileSystemEntries(current)
                            .OrderByDescending(item => item, StringComparer.Ordinal))
                            paths.Push(child);
                }
                input.FlushFinalBlock();
                return Convert.ToBase64String(algorithm.Hash);
            }
        }
    }
}
