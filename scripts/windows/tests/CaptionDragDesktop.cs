using System;
using System.ComponentModel;
using System.Runtime.InteropServices;

// Shared by the native caption probe and its supervising PowerShell process.
public static class MimiCaptionDragDesktop
{
    [DllImport("user32.dll", EntryPoint = "SystemParametersInfoW", SetLastError = true)]
    private static extern bool ReadSetting(uint action, uint parameter, out int value, uint flags);

    [DllImport("user32.dll", EntryPoint = "SystemParametersInfoW", SetLastError = true)]
    private static extern bool WriteSetting(uint action, uint parameter, IntPtr value, uint flags);

    public static bool FullWindows()
    {
        int value;
        if (!ReadSetting(0x0026, 0, out value, 0)) // SPI_GETDRAGFULLWINDOWS
            throw new Win32Exception(Marshal.GetLastWin32Error(), "Could not read the Windows drag preference.");
        return value != 0;
    }

    public static void SetFullWindows(bool enabled)
    {
        // SPI_SETDRAGFULLWINDOWS uses uiParam. Zero flags keep the change in this session.
        if (!WriteSetting(0x0025, enabled ? 1u : 0u, IntPtr.Zero, 0))
            throw new Win32Exception(Marshal.GetLastWin32Error(), "Could not set the test desktop's drag preference.");
        if (FullWindows() != enabled)
            throw new InvalidOperationException("The Windows drag preference did not read back as requested.");
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct MouseInput
    {
        public int X, Y;
        public uint Data, Flags, Time;
        public UIntPtr ExtraInfo;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct Input
    {
        public uint Type;
        public MouseInput Mouse;
    }

    [DllImport("user32.dll")]
    private static extern short GetAsyncKeyState(int key);

    [DllImport("user32.dll", SetLastError = true)]
    private static extern uint SendInput(uint count, Input[] inputs, int size);

    public static bool LeftDown()
    {
        return (GetAsyncKeyState(1) & 0x8000) != 0;
    }

    public static void SetLeftButton(bool down)
    {
        var input = new Input { Mouse = new MouseInput { Flags = down ? 0x0002u : 0x0004u } };
        if (SendInput(1, new[] { input }, Marshal.SizeOf(typeof(Input))) != 1)
            throw new Win32Exception(Marshal.GetLastWin32Error(), "Windows did not accept the caption drag button event.");
    }

    public static void Wheel(int delta)
    {
        // Mouse wheel data is a signed multiple of WHEEL_DELTA, packed in DWORD.
        if (delta == 0 || delta % 120 != 0)
            throw new ArgumentOutOfRangeException("delta", "Use whole physical wheel detents.");
        var input = new Input { Mouse = new MouseInput { Data = unchecked((uint)delta), Flags = 0x0800u } };
        if (SendInput(1, new[] { input }, Marshal.SizeOf(typeof(Input))) != 1)
            throw new Win32Exception(Marshal.GetLastWin32Error(), "Windows did not accept the history wheel event.");
    }
}
