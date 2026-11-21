using System;
using System.Collections.Generic;
using System.ComponentModel;
using System.Diagnostics;
using System.Runtime.InteropServices;
using System.Threading;

// Test-only native observer and opaque neutral backplate. No window titles,
// page text, network or input injection. Hook callbacks only read OS metadata.
public sealed class MimiPanelDragObservation : IDisposable
{
    [StructLayout(LayoutKind.Sequential)] public struct Rect {
        public int Left, Top, Right, Bottom;
        public int Width { get { return Right - Left; } }
        public int Height { get { return Bottom - Top; } }
    }
    [StructLayout(LayoutKind.Sequential)] private struct Point { public int X,Y; }
    [StructLayout(LayoutKind.Sequential)] private struct GuiInfo {
        public uint Size, Flags; public IntPtr Active, Focus, Capture, Menu, MoveSize, Caret; public Rect CaretRect;
    }
    [StructLayout(LayoutKind.Sequential)] private struct Message { public IntPtr Window; public uint Id; public UIntPtr WParam; public IntPtr LParam; public uint Time; public Point Position; public uint Private; }
    [StructLayout(LayoutKind.Sequential, CharSet=CharSet.Unicode)] private struct WindowClass {
        public uint Size, Style; public WindowProc Procedure; public int ClassExtra, WindowExtra; public IntPtr Instance, Icon, Cursor, Brush;
        public string Menu, Name; public IntPtr SmallIcon;
    }
    private delegate void WinEvent(IntPtr hook,uint id,IntPtr window,int objectId,int childId,uint thread,uint time);
    private delegate IntPtr WindowProc(IntPtr window,uint message,IntPtr wparam,IntPtr lparam);
    [DllImport("user32.dll")] private static extern IntPtr SetWinEventHook(uint min,uint max,IntPtr module,WinEvent callback,uint process,uint thread,uint flags);
    [DllImport("user32.dll")] private static extern bool UnhookWinEvent(IntPtr hook);
    [DllImport("user32.dll")] private static extern int GetMessage(out Message message,IntPtr window,uint min,uint max);
    [DllImport("user32.dll")] private static extern bool TranslateMessage(ref Message message);
    [DllImport("user32.dll")] private static extern IntPtr DispatchMessage(ref Message message);
    [DllImport("user32.dll")] private static extern bool PostThreadMessage(uint thread,uint message,UIntPtr wparam,IntPtr lparam);
    [DllImport("user32.dll")] private static extern uint GetWindowThreadProcessId(IntPtr window,out uint process);
    [DllImport("user32.dll")] private static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] private static extern IntPtr GetAncestor(IntPtr window,uint flags);
    [DllImport("user32.dll")] private static extern IntPtr WindowFromPoint(Point point);
    [DllImport("user32.dll")] private static extern bool GetWindowRect(IntPtr window,out Rect rect);
    [DllImport("user32.dll")] private static extern bool GetGUIThreadInfo(uint thread,ref GuiInfo info);
    [DllImport("user32.dll")] private static extern bool GetCursorPos(out Point point);
    [DllImport("user32.dll")] private static extern short GetAsyncKeyState(int key);
    [DllImport("user32.dll")] private static extern IntPtr SetThreadDpiAwarenessContext(IntPtr context);
    [DllImport("user32.dll",CharSet=CharSet.Unicode,SetLastError=true)] private static extern ushort RegisterClassEx(ref WindowClass value);
    [DllImport("user32.dll",CharSet=CharSet.Unicode,SetLastError=true)] private static extern IntPtr CreateWindowEx(uint ex,string cls,string title,uint style,int x,int y,int width,int height,IntPtr parent,IntPtr menu,IntPtr instance,IntPtr param);
    [DllImport("user32.dll")] private static extern IntPtr DefWindowProc(IntPtr window,uint message,IntPtr wparam,IntPtr lparam);
    [DllImport("user32.dll")] private static extern bool DestroyWindow(IntPtr window);
    [DllImport("user32.dll")] private static extern bool SetWindowPos(IntPtr window,IntPtr after,int x,int y,int width,int height,uint flags);
    [DllImport("user32.dll",CharSet=CharSet.Unicode)] private static extern bool UnregisterClass(string name,IntPtr instance);
    [DllImport("gdi32.dll")] private static extern IntPtr CreateSolidBrush(uint color);
    [DllImport("gdi32.dll")] private static extern bool DeleteObject(IntPtr value);
    [DllImport("kernel32.dll")] private static extern uint GetCurrentThreadId();
    [DllImport("kernel32.dll")] private static extern ulong GetTickCount64();
    [DllImport("kernel32.dll",CharSet=CharSet.Unicode)] private static extern IntPtr GetModuleHandle(string name);

    public sealed class Sample {
        public long HostTicks, UtcTicks; public ulong SystemTickMs; public double ElapsedMs;
        public uint Event, EventTickMs; public string Stage, Target, Foreground, Focus, Capture;
        public bool LeftDown, RootMoveModal; public Rect Root, Control; public int PointerX, PointerY;
    }
    private readonly IntPtr root,control; private readonly uint processId,rootThread;
    private readonly object sync=new object(); private readonly List<Sample> samples=new List<Sample>();
    private readonly ManualResetEventSlim ready=new ManualResetEventSlim(false); private readonly Thread thread;
    private readonly WinEvent callback; private readonly WindowProc procedure;
    private readonly long started=Stopwatch.GetTimestamp(); private string stage="installed"; private uint observerThread;
    private IntPtr backplate; private Exception failure; private bool disposed;
    public readonly Rect Region; public int DroppedSamples;
    public long Frequency { get { return Stopwatch.Frequency; } }
    public MimiPanelDragObservation(IntPtr rootWindow,IntPtr controlWindow) {
        root=rootWindow; control=controlWindow; rootThread=GetWindowThreadProcessId(root,out processId);
        Rect a,b; if(!GetWindowRect(root,out a)||!GetWindowRect(control,out b))throw new InvalidOperationException("Could not bound test windows.");
        Region=new Rect {Left=Math.Min(a.Left,b.Left)-24,Top=Math.Min(a.Top,b.Top)-24,Right=Math.Max(a.Right,b.Right)+72,Bottom=Math.Max(a.Bottom,b.Bottom)+24};
        if((Region.Width&1)!=0)Region=new Rect{Left=Region.Left,Top=Region.Top,Right=Region.Right+1,Bottom=Region.Bottom};
        if((Region.Height&1)!=0)Region=new Rect{Left=Region.Left,Top=Region.Top,Right=Region.Right,Bottom=Region.Bottom+1};
        callback=OnEvent; procedure=DefWindowProc; thread=new Thread(Run); thread.IsBackground=true; thread.SetApartmentState(ApartmentState.STA); thread.Start();
        if(!ready.Wait(5000))throw new InvalidOperationException("Native observer did not become ready.");
        if(failure!=null)throw new InvalidOperationException("Native observer setup failed.",failure);
        AssertPrivateRegion(); Mark("observer-ready");
    }
    private void Run() {
        var hooks=new List<IntPtr>(); IntPtr brush=IntPtr.Zero; string name="MimiPanelQaNeutral";
        try {
            observerThread=GetCurrentThreadId(); SetThreadDpiAwarenessContext(new IntPtr(-4));
            brush=CreateSolidBrush(0x181818); var cls=new WindowClass{Size=(uint)Marshal.SizeOf(typeof(WindowClass)),Procedure=procedure,Instance=GetModuleHandle(null),Brush=brush,Name=name};
            if(RegisterClassEx(ref cls)==0)throw new Win32Exception(Marshal.GetLastWin32Error());
            backplate=CreateWindowEx(0x08000080,name,"Mimi offline QA neutral background",0x90000000,Region.Left,Region.Top,Region.Width,Region.Height,IntPtr.Zero,IntPtr.Zero,cls.Instance,IntPtr.Zero);
            if(backplate==IntPtr.Zero||!SetWindowPos(backplate,new IntPtr(-1),Region.Left,Region.Top,Region.Width,Region.Height,0x0010)||!SetWindowPos(backplate,root,0,0,0,0,0x0013))throw new InvalidOperationException("Could not place neutral background below the application.");
            hooks.Add(SetWinEventHook(3,3,IntPtr.Zero,callback,0,0,0));
            hooks.Add(SetWinEventHook(10,11,IntPtr.Zero,callback,processId,0,0));
            hooks.Add(SetWinEventHook(0x8002,0x800B,IntPtr.Zero,callback,processId,0,0));
            foreach(var hook in hooks)if(hook==IntPtr.Zero)throw new InvalidOperationException("Native event observer could not subscribe.");
            ready.Set(); Message message; while(GetMessage(out message,IntPtr.Zero,0,0)>0){TranslateMessage(ref message);DispatchMessage(ref message);}
        } catch(Exception error){failure=error;ready.Set();}
        finally {foreach(var hook in hooks)if(hook!=IntPtr.Zero)UnhookWinEvent(hook);if(backplate!=IntPtr.Zero)DestroyWindow(backplate);if(brush!=IntPtr.Zero)DeleteObject(brush);UnregisterClass(name,GetModuleHandle(null));}
    }
    private string Category(IntPtr window) {
        if(window==IntPtr.Zero)return "none";var top=GetAncestor(window,2);
        return top==root?"root":top==control?"control":top==backplate?"qa-background":"other";
    }
    private void OnEvent(IntPtr hook,uint id,IntPtr window,int objectId,int childId,uint sourceThread,uint time) {
        var target=Category(window);
        if(id!=3&&target!="root"&&target!="control")return;
        if(id==0x800B&&(window!=root&&window!=control))return;
        if(id!=3&&id!=10&&id!=11&&id!=0x8002&&id!=0x8003&&id!=0x8005&&id!=0x800B)return;
        Record(id,time,target);
    }
    private void Record(uint id,uint time,string target) {
        var gui=new GuiInfo{Size=(uint)Marshal.SizeOf(typeof(GuiInfo))};GetGUIThreadInfo(rootThread,ref gui);
        Rect a,b;GetWindowRect(root,out a);GetWindowRect(control,out b);Point p;GetCursorPos(out p);long now=Stopwatch.GetTimestamp();
        lock(sync){if(samples.Count>=2048){DroppedSamples++;return;}samples.Add(new Sample{HostTicks=now,UtcTicks=DateTime.UtcNow.Ticks,SystemTickMs=GetTickCount64(),ElapsedMs=(now-started)*1000.0/Stopwatch.Frequency,Event=id,EventTickMs=time,Stage=stage,Target=target,Foreground=Category(GetForegroundWindow()),Focus=Category(gui.Focus),Capture=Category(gui.Capture),LeftDown=(GetAsyncKeyState(1)&0x8000)!=0,RootMoveModal=(gui.Flags&2)!=0&&gui.MoveSize==root,Root=a,Control=b,PointerX=p.X,PointerY=p.Y});}
    }
    public void Mark(string value){lock(sync)stage=value;Record(0,0,"mark");}
    public Sample[] Snapshot(){lock(sync)return samples.ToArray();}
    public void AssertPrivateRegion(){for(int y=Region.Top+2;y<Region.Bottom;y+=24)for(int x=Region.Left+2;x<Region.Right;x+=24){string owner=Category(WindowFromPoint(new Point{X=x,Y=y}));if(owner!="root"&&owner!="control"&&owner!="qa-background")throw new InvalidOperationException("Recording region contains a window outside the offline application and neutral background.");}}
    public void Dispose(){if(disposed)return;disposed=true;PostThreadMessage(observerThread,0x0012,UIntPtr.Zero,IntPtr.Zero);if(!thread.Join(5000))throw new InvalidOperationException("Native observer did not stop.");ready.Dispose();}
}
