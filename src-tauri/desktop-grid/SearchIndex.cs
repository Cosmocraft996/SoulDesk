using System;
using System.Collections.Generic;
using System.Data.OleDb;
using System.Diagnostics;
using System.IO;
using System.Linq;

namespace SoulDesk.DesktopGrid
{
    internal static class SearchIndex
    {
        private const int Limit = 100;

        internal static SearchBatch Find(string query, string[] local, bool isolated, bool indexed, string everything)
        {
            var result = new SearchBatch();
            var seen = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
            try
            {
                var paths = local.Concat(local.Where(Directory.Exists).Take(20).SelectMany(ReadDirectory));
                if (!isolated)
                    paths = paths.Concat(new[] { Environment.SpecialFolder.Programs, Environment.SpecialFolder.CommonPrograms }
                        .Select(Environment.GetFolderPath).Where(Directory.Exists).SelectMany(StartMenu));
                Add(result, seen, paths.Where(path => path.IndexOf(query, StringComparison.OrdinalIgnoreCase) >= 0));
            }
            catch (IOException) { }
            catch (UnauthorizedAccessException) { }
            if (everything != null)
            {
                try { Add(result, seen, Everything(everything, query)); }
                catch (Exception) { result.Note = "Everything 不可用"; }
            }
            if (indexed && (everything == null || result.Note != null) && result.Items.Count < Limit)
            {
                try { Add(result, seen, Windows(query)); }
                catch (Exception) { result.Note = "当前显示格子、桌面和开始菜单中的结果"; }
            }
            return result;
        }

        private static void Add(SearchBatch result, HashSet<string> seen, IEnumerable<string> paths)
        {
            foreach (string path in paths)
            {
                if (result.Items.Count == Limit) break;
                if (References.IsLocalPath(path) && seen.Add(path) && (File.Exists(path) || Directory.Exists(path)))
                    result.Items.Add(new SearchResult { Path = path });
            }
        }

        private static IEnumerable<string> ReadDirectory(string folder)
        {
            try { return Directory.EnumerateFileSystemEntries(folder).Take(400).ToArray(); }
            catch (IOException) { return new string[0]; }
            catch (UnauthorizedAccessException) { return new string[0]; }
        }

        private static IEnumerable<string> StartMenu(string root)
        {
            var pending = new Queue<string>();
            pending.Enqueue(root);
            int visited = 0;
            while (pending.Count != 0 && visited++ < 120)
                foreach (string path in ReadDirectory(pending.Dequeue()))
                {
                    if (Directory.Exists(path))
                    {
                        if ((File.GetAttributes(path) & FileAttributes.ReparsePoint) == 0)
                            pending.Enqueue(path);
                    }
                    else if (path.EndsWith(".lnk", StringComparison.OrdinalIgnoreCase))
                        yield return path;
                }
        }

        private static List<string> Windows(string query)
        {
            string escaped = query.Replace("'", "''").Replace("[", "[[]").Replace("%", "[%]").Replace("_", "[_]");
            var result = new List<string>();
            using (var connection = new OleDbConnection("Provider=Search.CollatorDSO;Extended Properties='Application=Windows';"))
            {
                connection.Open();
                using (var command = connection.CreateCommand())
                {
                    command.CommandTimeout = 4;
                    command.CommandText = "SELECT TOP 100 System.ItemPathDisplay FROM SYSTEMINDEX WHERE System.ItemNameDisplay LIKE '%" +
                        escaped + "%' OR System.ItemPathDisplay LIKE '%" + escaped + "%'";
                    using (var rows = command.ExecuteReader())
                        while (rows.Read())
                            if (rows.GetValue(0) is string) result.Add(rows.GetString(0));
                }
            }
            return result;
        }

        private static string[] Everything(string executable, string query)
        {
            if (!File.Exists(executable)) throw new FileNotFoundException();
            if (query.IndexOfAny(new[] { '"', '\r', '\n', '<', '>', '|', ':' }) >= 0 ||
                query.StartsWith("-") || query.StartsWith("/"))
                return new string[0];
            // ES 的控制台输出受系统代码页影响。文件导出保留 UTF-8 路径。
            string output = Path.Combine(Path.GetTempPath(), "SoulDesk-ES-" + Guid.NewGuid().ToString("N") + ".txt");
            try
            {
                string args = "-n 100 -export-txt \"" + output + "\" \"" + query.Replace("\\", "\\\\") + "\"";
                using (var process = Process.Start(new ProcessStartInfo(executable, args)
                {
                    CreateNoWindow = true, UseShellExecute = false
                }))
                {
                    if (!process.WaitForExit(2800))
                    {
                        process.Kill();
                        throw new TimeoutException();
                    }
                    if (process.ExitCode != 0)
                        throw new IOException();
                }
                return File.ReadAllLines(output, System.Text.Encoding.UTF8)
                    .Where(line => line.Length != 0).Select(line => line.Trim('"')).ToArray();
            }
            finally
            {
                try { File.Delete(output); }
                catch (IOException) { }
                catch (UnauthorizedAccessException) { }
            }
        }
    }
}
