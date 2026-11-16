$ErrorActionPreference = 'Stop'

if ($env:RUNNER_ENVIRONMENT -ne 'github-hosted' -or $env:RUNNER_ARCH -ne 'ARM64' -or
    [Runtime.InteropServices.RuntimeInformation]::ProcessArchitecture -ne 'Arm64') {
    throw 'The endpoint probe requires a native hosted Windows ARM64 process.'
}

$services = @(Get-Service -Name AudioSrv,AudioEndpointBuilder -ErrorAction SilentlyContinue | Where-Object Status -eq Running).Count
Write-Output "Windows audio services running: $services of 2."

Add-Type @'
using System;
using System.Runtime.InteropServices;

public static class MimiDefaultAudioEndpointProbe {
    [ComImport]
    [Guid("A95664D2-9614-4F35-A746-DE8DB63617E6")]
    [InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    private interface IMMDeviceEnumerator {
        [PreserveSig]
        int EnumAudioEndpoints(int dataFlow, uint states, out IntPtr devices);
        [PreserveSig]
        int GetDefaultAudioEndpoint(int dataFlow, int role, out IntPtr endpoint);
    }

    public static void Run() {
        var clsid = new Guid("BCDE0395-E52F-467C-8E3D-C4579291692E");
        var instance = Activator.CreateInstance(Type.GetTypeFromCLSID(clsid, true));
        try {
            var enumerator = (IMMDeviceEnumerator)instance;
            var roles = new[] { "console", "multimedia", "communications" };
            for (int role = 0; role < roles.Length; role++) {
                IntPtr endpoint;
                int status = enumerator.GetDefaultAudioEndpoint(0, role, out endpoint); // eRender
                var available = status == 0 && endpoint != IntPtr.Zero;
                if (endpoint != IntPtr.Zero) Marshal.Release(endpoint);
                Console.WriteLine("WASAPI render role={0} status=0x{1:X8} endpoint={2}", roles[role], status, available);
            }
        } finally {
            Marshal.ReleaseComObject(instance);
        }
    }
}
'@
[MimiDefaultAudioEndpointProbe]::Run()
