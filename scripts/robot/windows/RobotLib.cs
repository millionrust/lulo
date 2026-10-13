// The Windows robot's native helpers: real input, windows, screenshots,
// screenshot analysis, process numbers and UI Automation.
//
// robot.ps1 compiles this with Add-Type (Windows PowerShell 5.1, so C# 5:
// no string interpolation, no `out var`, no expression-bodied members).
// See README.md for what the robot does with it.
using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Drawing;
using System.Drawing.Imaging;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;
using System.Windows.Automation;

namespace LuloRobot
{
    [StructLayout(LayoutKind.Sequential)]
    public struct RECT { public int Left, Top, Right, Bottom; }

    public static class Native
    {
        [StructLayout(LayoutKind.Sequential)]
        public struct LASTINPUTINFO { public uint cbSize; public uint dwTime; }

        [StructLayout(LayoutKind.Sequential)]
        public struct MOUSEINPUT { public int dx, dy; public uint mouseData, dwFlags, time; public IntPtr dwExtraInfo; }
        [StructLayout(LayoutKind.Sequential)]
        public struct KEYBDINPUT { public ushort wVk, wScan; public uint dwFlags, time; public IntPtr dwExtraInfo; }
        [StructLayout(LayoutKind.Explicit)]
        public struct INPUTUNION { [FieldOffset(0)] public MOUSEINPUT mi; [FieldOffset(0)] public KEYBDINPUT ki; }
        [StructLayout(LayoutKind.Sequential)]
        public struct INPUT { public uint type; public INPUTUNION u; }

        [StructLayout(LayoutKind.Sequential)]
        public struct PROCESS_MEMORY_COUNTERS_EX
        {
            public uint cb, PageFaultCount;
            public UIntPtr PeakWorkingSetSize, WorkingSetSize, QuotaPeakPagedPoolUsage, QuotaPagedPoolUsage,
                QuotaPeakNonPagedPoolUsage, QuotaNonPagedPoolUsage, PagefileUsage, PeakPagefileUsage, PrivateUsage;
        }

        public delegate bool EnumWindowsProc(IntPtr hwnd, IntPtr lParam);

        [DllImport("user32.dll")] public static extern bool GetLastInputInfo(ref LASTINPUTINFO info);
        [DllImport("kernel32.dll")] public static extern uint GetTickCount();
        [DllImport("user32.dll", SetLastError = true)] public static extern uint SendInput(uint n, INPUT[] inputs, int size);
        [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
        [DllImport("user32.dll")] public static extern bool EnumWindows(EnumWindowsProc proc, IntPtr lParam);
        [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint pid);
        [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr hwnd);
        [DllImport("user32.dll")] public static extern bool IsWindow(IntPtr hwnd);
        [DllImport("user32.dll")] public static extern bool IsIconic(IntPtr hwnd);
        [DllImport("user32.dll")] public static extern bool IsHungAppWindow(IntPtr hwnd);
        [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetWindowText(IntPtr hwnd, StringBuilder text, int max);
        [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetClassName(IntPtr hwnd, StringBuilder text, int max);
        [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hwnd, out RECT rect);
        [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
        [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hwnd);
        [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr hwnd, IntPtr after, int x, int y, int cx, int cy, uint flags);
        [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr hwnd, int cmd);
        [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr hwnd, uint msg, IntPtr w, IntPtr l);
        [DllImport("user32.dll")] public static extern int GetWindowLong(IntPtr hwnd, int index);
        [DllImport("user32.dll")] public static extern IntPtr GetWindow(IntPtr hwnd, uint cmd);
        [DllImport("user32.dll")] public static extern bool SystemParametersInfo(uint action, uint param, ref RECT rect, uint winIni);
        [DllImport("user32.dll")] public static extern IntPtr FindWindow(string cls, string title);
        [DllImport("user32.dll")] public static extern int GetSystemMetrics(int index);
        [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
        [DllImport("user32.dll")] public static extern IntPtr MonitorFromWindow(IntPtr hwnd, uint flags);
        [DllImport("dwmapi.dll")] public static extern int DwmGetWindowAttribute(IntPtr hwnd, int attr, out RECT rect, int size);
        [DllImport("dwmapi.dll")] public static extern int DwmGetWindowAttribute(IntPtr hwnd, int attr, out int value, int size);
        [DllImport("psapi.dll")] public static extern bool GetProcessMemoryInfo(IntPtr process, out PROCESS_MEMORY_COUNTERS_EX counters, int size);
        [DllImport("user32.dll")] public static extern uint GetGuiResources(IntPtr process, uint flags);
    }

    public static class Robot
    {
        public const int INPUT_MOUSE = 0, INPUT_KEYBOARD = 1;
        const uint KEYEVENTF_EXTENDEDKEY = 1, KEYEVENTF_KEYUP = 2;
        const uint MOUSEEVENTF_MOVE = 1, LEFTDOWN = 2, LEFTUP = 4, RIGHTDOWN = 8, RIGHTUP = 0x10,
            WHEEL = 0x800, ABSOLUTE = 0x8000, VIRTUALDESK = 0x4000;

        static Robot() { Native.SetProcessDPIAware(); }

        // Milliseconds since the last real input in this session.
        public static uint IdleMs()
        {
            Native.LASTINPUTINFO info = new Native.LASTINPUTINFO();
            info.cbSize = (uint)Marshal.SizeOf(typeof(Native.LASTINPUTINFO));
            Native.GetLastInputInfo(ref info);
            return unchecked(Native.GetTickCount() - info.dwTime);
        }

        static void Send(Native.INPUT[] inputs)
        {
            Native.SendInput((uint)inputs.Length, inputs, Marshal.SizeOf(typeof(Native.INPUT)));
        }

        static Native.INPUT KeyInput(ushort vk, bool up)
        {
            Native.INPUT input = new Native.INPUT();
            input.type = INPUT_KEYBOARD;
            input.u.ki.wVk = vk;
            uint flags = up ? KEYEVENTF_KEYUP : 0;
            // Arrows, Home/End, PgUp/PgDn, Ins/Del and the Windows keys are extended.
            if ((vk >= 0x21 && vk <= 0x2E) || vk == 0x5B || vk == 0x5C) flags |= KEYEVENTF_EXTENDEDKEY;
            input.u.ki.dwFlags = flags;
            return input;
        }

        public static void KeyDown(int vk) { Send(new[] { KeyInput((ushort)vk, false) }); Thread.Sleep(30); }
        public static void KeyUp(int vk) { Send(new[] { KeyInput((ushort)vk, true) }); Thread.Sleep(30); }

        // Press the keys in order and release them in reverse: Chord(0x11, 0x4E) is Ctrl+N.
        public static void Chord(params int[] keys)
        {
            foreach (int k in keys) KeyDown(k);
            for (int i = keys.Length - 1; i >= 0; i--) KeyUp(keys[i]);
            Thread.Sleep(60);
        }

        public static void Type(string text)
        {
            foreach (char c in text)
            {
                Native.INPUT down = new Native.INPUT(); down.type = INPUT_KEYBOARD;
                down.u.ki.wScan = c; down.u.ki.dwFlags = 4; // KEYEVENTF_UNICODE
                Native.INPUT up = down; up.u.ki.dwFlags = 4 | KEYEVENTF_KEYUP;
                Send(new[] { down, up });
                Thread.Sleep(25);
            }
        }

        static Native.INPUT Mouse(uint flags, int x, int y, uint data)
        {
            int vx = Native.GetSystemMetrics(76), vy = Native.GetSystemMetrics(77);
            int vw = Math.Max(1, Native.GetSystemMetrics(78) - 1), vh = Math.Max(1, Native.GetSystemMetrics(79) - 1);
            Native.INPUT input = new Native.INPUT();
            input.type = INPUT_MOUSE;
            input.u.mi.dx = (int)Math.Round((x - vx) * 65535.0 / vw);
            input.u.mi.dy = (int)Math.Round((y - vy) * 65535.0 / vh);
            input.u.mi.mouseData = data;
            input.u.mi.dwFlags = flags | ABSOLUTE | VIRTUALDESK | MOUSEEVENTF_MOVE;
            return input;
        }

        public static void MoveTo(int x, int y) { Send(new[] { Mouse(0, x, y, 0) }); Thread.Sleep(80); }

        public static void Click(int x, int y) { MoveTo(x, y); Send(new[] { Mouse(LEFTDOWN, x, y, 0) }); Thread.Sleep(40); Send(new[] { Mouse(LEFTUP, x, y, 0) }); Thread.Sleep(80); }
        public static void RightClick(int x, int y) { MoveTo(x, y); Send(new[] { Mouse(RIGHTDOWN, x, y, 0) }); Thread.Sleep(40); Send(new[] { Mouse(RIGHTUP, x, y, 0) }); Thread.Sleep(80); }
        public static void DoubleClick(int x, int y)
        {
            MoveTo(x, y);
            for (int i = 0; i < 2; i++) { Send(new[] { Mouse(LEFTDOWN, x, y, 0) }); Thread.Sleep(30); Send(new[] { Mouse(LEFTUP, x, y, 0) }); Thread.Sleep(60); }
        }
        public static void Wheel(int x, int y, int clicks) { MoveTo(x, y); Send(new[] { Mouse(WHEEL, x, y, unchecked((uint)(clicks * 120))) }); Thread.Sleep(80); }
        public static void Drag(int x0, int y0, int x1, int y1)
        {
            MoveTo(x0, y0); Send(new[] { Mouse(LEFTDOWN, x0, y0, 0) }); Thread.Sleep(80);
            for (int i = 1; i <= 10; i++) { MoveTo(x0 + (x1 - x0) * i / 10, y0 + (y1 - y0) * i / 10); }
            Send(new[] { Mouse(LEFTUP, x1, y1, 0) }); Thread.Sleep(120);
        }

        // ---- Windows -------------------------------------------------------

        public static IntPtr[] WindowsOf(int pid, bool titled)
        {
            List<IntPtr> found = new List<IntPtr>();
            Native.EnumWindows(delegate (IntPtr h, IntPtr l)
            {
                uint owner; Native.GetWindowThreadProcessId(h, out owner);
                if (owner == (uint)pid && Native.IsWindowVisible(h) && (!titled || Title(h).Length > 0)) found.Add(h);
                return true;
            }, IntPtr.Zero);
            return found.ToArray();
        }

        // Every visible top-level window, top of the z-order first.
        public static IntPtr[] ZOrder()
        {
            List<IntPtr> found = new List<IntPtr>();
            Native.EnumWindows(delegate (IntPtr h, IntPtr l) { if (Native.IsWindowVisible(h)) found.Add(h); return true; }, IntPtr.Zero);
            return found.ToArray();
        }

        public static string Title(IntPtr h) { StringBuilder b = new StringBuilder(512); Native.GetWindowText(h, b, 512); return b.ToString(); }
        public static string ClassOf(IntPtr h) { StringBuilder b = new StringBuilder(256); Native.GetClassName(h, b, 256); return b.ToString(); }
        public static int PidOf(IntPtr h) { uint p; Native.GetWindowThreadProcessId(h, out p); return (int)p; }
        public static bool IsHung(IntPtr h) { return Native.IsHungAppWindow(h); }
        public static bool Alive(IntPtr h) { return Native.IsWindow(h); }
        public static bool Minimized(IntPtr h) { return Native.IsIconic(h); }
        public static IntPtr Foreground() { return Native.GetForegroundWindow(); }
        public static bool Activate(IntPtr h) { return Native.SetForegroundWindow(h); }
        public static void Close(IntPtr h) { Native.PostMessage(h, 0x0010, IntPtr.Zero, IntPtr.Zero); }
        public static void Show(IntPtr h, int cmd) { Native.ShowWindow(h, cmd); }

        // The visible frame (DWM's extended frame bounds, without the invisible resize borders).
        public static Rectangle Frame(IntPtr h)
        {
            RECT r;
            if (Native.DwmGetWindowAttribute(h, 9, out r, Marshal.SizeOf(typeof(RECT))) != 0) Native.GetWindowRect(h, out r);
            return Rectangle.FromLTRB(r.Left, r.Top, r.Right, r.Bottom);
        }

        public static Rectangle WindowRect(IntPtr h) { RECT r; Native.GetWindowRect(h, out r); return Rectangle.FromLTRB(r.Left, r.Top, r.Right, r.Bottom); }

        // Put the window's visible frame at x, y, w, hgt (SetWindowPos counts the invisible resize borders too).
        public static bool Move(IntPtr h, int x, int y, int w, int hgt)
        {
            Rectangle f = Frame(h), r = WindowRect(h);
            int l = f.Left - r.Left, t = f.Top - r.Top, rr = r.Right - f.Right, b = r.Bottom - f.Bottom;
            return Native.SetWindowPos(h, IntPtr.Zero, x - l, y - t, w + l + rr, hgt + t + b, 0x0004 | 0x0010);
        }

        public static Rectangle WorkArea() { RECT r = new RECT(); Native.SystemParametersInfo(0x30, 0, ref r, 0); return Rectangle.FromLTRB(r.Left, r.Top, r.Right, r.Bottom); }
        public static Rectangle Screen() { return new Rectangle(0, 0, Native.GetSystemMetrics(0), Native.GetSystemMetrics(1)); }

        public static IntPtr Taskbar() { return Native.FindWindow("Shell_TrayWnd", null); }
        public static bool Visible(IntPtr h) { return h != IntPtr.Zero && Native.IsWindowVisible(h); }
        public static bool Topmost(IntPtr h) { return (Native.GetWindowLong(h, -20) & 0x8) != 0; }
        // WS_THICKFRAME: the user can resize the window.
        public static bool Resizable(IntPtr h) { return (Native.GetWindowLong(h, -16) & 0x40000) != 0; }

        // ---- Processes -----------------------------------------------------

        public static long PrivateBytes(int pid)
        {
            try
            {
                using (Process p = Process.GetProcessById(pid))
                {
                    Native.PROCESS_MEMORY_COUNTERS_EX c;
                    if (Native.GetProcessMemoryInfo(p.Handle, out c, Marshal.SizeOf(typeof(Native.PROCESS_MEMORY_COUNTERS_EX))))
                        return (long)c.PrivateUsage.ToUInt64();
                }
            }
            catch (Exception) { }
            return -1;
        }

        public static double CpuMs(int pid)
        {
            try { using (Process p = Process.GetProcessById(pid)) { return p.TotalProcessorTime.TotalMilliseconds; } }
            catch (Exception) { return -1; }
        }

        // GDI and USER objects: a leak shows here before memory does.
        public static string GuiObjects(int pid)
        {
            try { using (Process p = Process.GetProcessById(pid)) { return Native.GetGuiResources(p.Handle, 0) + "/" + Native.GetGuiResources(p.Handle, 1); } }
            catch (Exception) { return ""; }
        }

        // Start an exe with extra environment variables (the private profile
        // folders), its stderr copied to `errPath` as it arrives (a panic's
        // message is the crash report).
        public static Process Launch(string exe, string args, string workDir, string[] env, string errPath)
        {
            ProcessStartInfo info = new ProcessStartInfo(exe, args ?? "");
            info.UseShellExecute = false;
            info.WorkingDirectory = workDir;
            info.RedirectStandardError = true;
            info.RedirectStandardOutput = true;
            for (int i = 0; i + 1 < env.Length; i += 2) info.EnvironmentVariables[env[i]] = env[i + 1];
            Process p = new Process();
            p.StartInfo = info;
            System.IO.StreamWriter w = new System.IO.StreamWriter(errPath, false, Encoding.UTF8);
            w.AutoFlush = true;
            p.ErrorDataReceived += delegate (object s, DataReceivedEventArgs e) { if (e.Data != null) lock (w) { w.WriteLine(e.Data); } };
            p.OutputDataReceived += delegate (object s, DataReceivedEventArgs e) { };
            p.Exited += delegate (object s, EventArgs e) { lock (w) { w.Flush(); } };
            p.EnableRaisingEvents = true;
            p.Start();
            p.BeginErrorReadLine();
            p.BeginOutputReadLine();
            return p;
        }

        // ---- Screenshots ---------------------------------------------------

        public static Bitmap Grab(Rectangle r)
        {
            Rectangle s = new Rectangle(Native.GetSystemMetrics(76), Native.GetSystemMetrics(77), Native.GetSystemMetrics(78), Native.GetSystemMetrics(79));
            r.Intersect(s);
            if (r.Width <= 0 || r.Height <= 0) return null;
            Bitmap b = new Bitmap(r.Width, r.Height, PixelFormat.Format32bppArgb);
            using (Graphics g = Graphics.FromImage(b)) { g.CopyFromScreen(r.Location, Point.Empty, r.Size); }
            return b;
        }

        public static void Save(Bitmap b, string path)
        {
            if (b == null) return;
            if (path.EndsWith(".jpg", StringComparison.OrdinalIgnoreCase))
            {
                ImageCodecInfo jpeg = null;
                foreach (ImageCodecInfo c in ImageCodecInfo.GetImageEncoders()) if (c.MimeType == "image/jpeg") jpeg = c;
                EncoderParameters p = new EncoderParameters(1);
                p.Param[0] = new EncoderParameter(System.Drawing.Imaging.Encoder.Quality, 85L);
                b.Save(path, jpeg, p);
                return;
            }
            b.Save(path, ImageFormat.Png);
        }

        // A capture of the whole screen saved straight to a file.
        public static void Shot(string path) { using (Bitmap b = Grab(Screen())) { Save(b, path); } }

        static int[] Pixels(Bitmap b)
        {
            Rectangle r = new Rectangle(0, 0, b.Width, b.Height);
            BitmapData d = b.LockBits(r, ImageLockMode.ReadOnly, PixelFormat.Format32bppArgb);
            int[] px = new int[b.Width * b.Height];
            for (int y = 0; y < b.Height; y++) Marshal.Copy(IntPtr.Add(d.Scan0, y * d.Stride), px, y * b.Width, b.Width);
            b.UnlockBits(d);
            return px;
        }

        static double Lum(int c)
        {
            int r = (c >> 16) & 255, g = (c >> 8) & 255, bl = c & 255;
            return 0.2126 * r + 0.7152 * g + 0.0722 * bl;
        }

        // The share of pixels that differ (by more than a small tolerance) between two captures of the same size.
        public static double Diff(Bitmap a, Bitmap b)
        {
            if (a == null || b == null) return 1.0;
            if (a.Width != b.Width || a.Height != b.Height) return 1.0;
            int[] pa = Pixels(a), pb = Pixels(b);
            int changed = 0;
            for (int i = 0; i < pa.Length; i++)
            {
                int x = pa[i], y = pb[i];
                if (Math.Abs(((x >> 16) & 255) - ((y >> 16) & 255)) + Math.Abs(((x >> 8) & 255) - ((y >> 8) & 255)) + Math.Abs((x & 255) - (y & 255)) > 24) changed++;
            }
            return (double)changed / pa.Length;
        }

        // A summary of one capture, for the visual checks in robot.ps1:
        // the share of near-black pixels, the share of the most common colour
        // (a blank window is one colour), mean luminance, the spread between
        // the darkest and lightest 2 % (low contrast), and the number of
        // distinct colours after dropping the low 3 bits.
        public static Dictionary<string, double> Analyze(Bitmap b)
        {
            Dictionary<string, double> o = new Dictionary<string, double>();
            if (b == null) return o;
            int[] px = Pixels(b);
            int black = 0; double sum = 0;
            Dictionary<int, int> counts = new Dictionary<int, int>();
            int[] hist = new int[256];
            for (int i = 0; i < px.Length; i++)
            {
                int c = px[i] & 0xF8F8F8;
                int n; counts.TryGetValue(c, out n); counts[c] = n + 1;
                double l = Lum(px[i]);
                sum += l; hist[(int)l]++;
                if (l < 12) black++;
            }
            int top = 0; foreach (int v in counts.Values) if (v > top) top = v;
            int lo = 0, hi = 255, acc = 0, cut = px.Length / 200;
            for (int i = 0; i < 256; i++) { acc += hist[i]; if (acc > cut) { lo = i; break; } }
            acc = 0;
            for (int i = 255; i >= 0; i--) { acc += hist[i]; if (acc > cut) { hi = i; break; } }
            o["black"] = (double)black / px.Length;
            o["dominant"] = (double)top / px.Length;
            o["mean"] = sum / px.Length;
            o["spread"] = hi - lo;
            o["colours"] = counts.Count;
            return o;
        }

        // ---- UI Automation -------------------------------------------------

        public class Node
        {
            public string Name, Type, AutomationId;
            public Rectangle Bounds;
            public bool Enabled, Offscreen;
            public int Depth;
            public AutomationElement Element;
        }

        // The UIA elements under a window, depth first, with their names and bounds.
        public static List<Node> Tree(IntPtr hwnd, int maxDepth, int maxNodes)
        {
            List<Node> nodes = new List<Node>();
            try
            {
                AutomationElement root = AutomationElement.FromHandle(hwnd);
                Walk(root, 0, maxDepth, maxNodes, nodes);
            }
            catch (Exception) { }
            return nodes;
        }

        static void Walk(AutomationElement e, int depth, int maxDepth, int maxNodes, List<Node> nodes)
        {
            if (nodes.Count >= maxNodes) return;
            Node n = new Node();
            try
            {
                AutomationElement.AutomationElementInformation c = e.Current;
                n.Name = c.Name ?? ""; n.Type = c.ControlType == null ? "" : c.ControlType.ProgrammaticName.Replace("ControlType.", "");
                n.AutomationId = c.AutomationId ?? "";
                System.Windows.Rect r = c.BoundingRectangle;
                n.Bounds = r.IsEmpty ? Rectangle.Empty : new Rectangle((int)r.X, (int)r.Y, (int)r.Width, (int)r.Height);
                n.Enabled = c.IsEnabled; n.Offscreen = c.IsOffscreen; n.Depth = depth; n.Element = e;
            }
            catch (Exception) { return; }
            nodes.Add(n);
            if (depth >= maxDepth) return;
            try
            {
                AutomationElement child = TreeWalker.RawViewWalker.GetFirstChild(e);
                while (child != null && nodes.Count < maxNodes)
                {
                    Walk(child, depth + 1, maxDepth, maxNodes, nodes);
                    child = TreeWalker.RawViewWalker.GetNextSibling(child);
                }
            }
            catch (Exception) { }
        }

        // Invoke (or select, or toggle) an element the way a screen reader would.
        public static string Invoke(AutomationElement e)
        {
            object p;
            try
            {
                if (e.TryGetCurrentPattern(InvokePattern.Pattern, out p)) { ((InvokePattern)p).Invoke(); return "invoke"; }
                if (e.TryGetCurrentPattern(SelectionItemPattern.Pattern, out p)) { ((SelectionItemPattern)p).Select(); return "select"; }
                if (e.TryGetCurrentPattern(TogglePattern.Pattern, out p)) { ((TogglePattern)p).Toggle(); return "toggle"; }
                if (e.TryGetCurrentPattern(ExpandCollapsePattern.Pattern, out p)) { ((ExpandCollapsePattern)p).Expand(); return "expand"; }
            }
            catch (Exception ex) { return "error: " + ex.Message; }
            return "";
        }
    }
}
