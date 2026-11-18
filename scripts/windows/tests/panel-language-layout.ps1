# Dot-sourced by the isolated, per-monitor-DPI-aware Windows UIA probe.
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes, WindowsBase, System.Drawing

if (-not ('MimiPanelCrop' -as [type])) {
    Add-Type -ReferencedAssemblies ([Drawing.Bitmap].Assembly.Location) -TypeDefinition @'
using System;
using System.Drawing;
using System.Drawing.Imaging;
using System.Runtime.InteropServices;

public static class MimiPanelCrop {
    [StructLayout(LayoutKind.Sequential)]
    private struct Rect { public int Left, Top, Right, Bottom; }
    [DllImport("user32.dll")] private static extern IntPtr GetWindow(IntPtr window, uint command);
    [DllImport("user32.dll")] private static extern bool IsWindowVisible(IntPtr window);
    [DllImport("user32.dll")] private static extern bool GetWindowRect(IntPtr window, out Rect rect);
    [DllImport("dwmapi.dll")] private static extern int DwmGetWindowAttribute(IntPtr window, int attribute, out int value, int size);
    [DllImport("user32.dll")] private static extern uint GetDpiForWindow(IntPtr window);

    public static void AssertUnobscured(IntPtr target, Rectangle crop) {
        for (var window = GetWindow(target, 3); window != IntPtr.Zero; window = GetWindow(window, 3)) { // GW_HWNDPREV
            Rect rect; int cloaked;
            if (IsWindowVisible(window) && GetWindowRect(window, out rect) && rect.Right > rect.Left && rect.Bottom > rect.Top
                && !(DwmGetWindowAttribute(window, 14, out cloaked, 4) == 0 && cloaked != 0)
                && crop.IntersectsWith(Rectangle.FromLTRB(rect.Left, rect.Top, rect.Right, rect.Bottom)))
                throw new InvalidOperationException("Another visible window overlaps the panel capture.");
        }
    }

    public static void Save(IntPtr window, Rectangle crop, int[] pixels, string path) {
        if (pixels.Length != crop.Width * crop.Height) throw new InvalidOperationException("Incomplete panel pixels.");
        // The WebView is transparent outside its 13-CSS-pixel rounded panel.
        // Discard these corner pixels, including the mixed antialias edge.
        int radius = (int)Math.Ceiling(13.0 * GetDpiForWindow(window) / 96.0);
        for (int y = 0; y < crop.Height; y++) for (int x = 0; x < crop.Width; x++) {
            double dx = Math.Max(0, radius - Math.Min(x + .5, crop.Width - x - .5));
            double dy = Math.Max(0, radius - Math.Min(y + .5, crop.Height - y - .5));
            int index = y * crop.Width + x;
            bool outside = dx > 0 && dy > 0 && dx * dx + dy * dy >= (radius - 1) * (radius - 1);
            pixels[index] = outside ? 0 : pixels[index] | unchecked((int)0xff000000);
        }
        using (var image = new Bitmap(crop.Width, crop.Height, PixelFormat.Format32bppArgb)) {
            var bits = image.LockBits(new Rectangle(0, 0, crop.Width, crop.Height), ImageLockMode.WriteOnly, image.PixelFormat);
            try { Marshal.Copy(pixels, 0, bits.Scan0, pixels.Length); }
            finally { image.UnlockBits(bits); }
            image.Save(path, ImageFormat.Png);
        }
    }
}
'@
}

function Assert-PanelBounds([System.Windows.Rect]$Box, [System.Windows.Rect]$Container, [string]$Key) {
    if ($Box.IsEmpty -or -not ($Box.Width -gt 0 -and $Box.Height -gt 0) -or
        $Box.Left -lt $Container.Left - 1 -or $Box.Right -gt $Container.Right + 1 -or
        $Box.Top -lt $Container.Top - 1 -or $Box.Bottom -gt $Container.Bottom + 1) {
        throw "The $Key bounds are empty or clipped."
    }
}

function Get-PanelToggleState($Element, [string]$Key) {
    $pattern = $null
    if ($Element.TryGetCurrentPattern([System.Windows.Automation.TogglePattern]::Pattern, [ref]$pattern)) {
        switch ($pattern.Current.ToggleState.ToString()) {
            'On' { return $true }
            'Off' { return $false }
        }
    }
    if ($Element.TryGetCurrentPattern([System.Windows.Automation.SelectionItemPattern]::Pattern, [ref]$pattern)) {
        return $pattern.Current.IsSelected
    }
    throw "The $Key control did not expose its selected state."
}

function Get-PanelPickerValueEvidence($Element, [string]$Expected, [string]$Key) {
    $scope = [System.Windows.Automation.TreeScope]::Descendants
    $text = [System.Windows.Automation.PropertyCondition]::new(
        [System.Windows.Automation.AutomationElement]::ControlTypeProperty, [System.Windows.Automation.ControlType]::Text)
    $children = @($Element.FindAll($scope, $text) | Where-Object { -not $_.Current.IsOffscreen -and -not [string]::IsNullOrWhiteSpace($_.Current.Name) })
    if ($children.Count) {
        $matching = @($children | Where-Object { $_.Current.Name -ceq $Expected })
        if ($matching.Count -eq 0) { throw "The $Key picker shows an unexpected value." }
        foreach ($child in $matching) { Assert-PanelBounds $child.Current.BoundingRectangle $Element.Current.BoundingRectangle "$Key text" }
        return 'text bounds'
    }
    $pattern = $null
    if ($Element.TryGetCurrentPattern([System.Windows.Automation.ValuePattern]::Pattern, [ref]$pattern) -and $pattern.Current.Value) {
        if ($pattern.Current.Value -cne $Expected) { throw "The $Key accessible value differs from the fixture." }
        return 'accessible value only'
    }
    if ($Element.TryGetCurrentPattern([System.Windows.Automation.SelectionPattern]::Pattern, [ref]$pattern)) {
        $selected = $pattern.GetCurrentSelection()
        if ($selected.Count) {
            if ($selected.Count -ne 1 -or $selected[0].Current.Name -cne $Expected) { throw "The $Key picker selection differs from the fixture." }
            return 'selected item only'
        }
    }
    # A renderer may fold the value into the combobox without a UIA text node.
    # The visible value then requires inspection in the original PNG.
    return 'review PNG'
}

function Save-PanelLanguageEvidence([IntPtr]$Controls, $Labels, [string]$EvidencePath) {
    $locale = [string]$Labels.locale
    if (@('zh', 'zh-TW', 'en', 'ja', 'ko', 'de', 'fr', 'th', 'ru') -cnotcontains $locale -or @($Labels.modes).Count -ne 3) {
        throw 'Invalid panel language fixture.'
    }
    $window = [MimiPointerSmoke]::ReadBounds($Controls)
    $frame = [System.Windows.Rect]::new($window.Left, $window.Top, $window.Width, $window.Height)
    $root = [System.Windows.Automation.AutomationElement]::FromHandle($Controls)
    $scope = [System.Windows.Automation.TreeScope]::Descendants
    $dialogId = [System.Windows.Automation.PropertyCondition]::new(
        [System.Windows.Automation.AutomationElement]::AutomationIdProperty, 'overlay-control-panel')
    $dialog = $root.FindFirst($scope, $dialogId)
    $surface = $root
    $panel = $frame
    if ($null -ne $dialog) {
        $surface = $dialog
        $panel = $dialog.Current.BoundingRectangle
        Assert-PanelBounds $panel $frame 'dialog'
    }
    $crop = [Drawing.Rectangle]::FromLTRB([Math]::Ceiling([Math]::Max($frame.Left, $panel.Left)), [Math]::Ceiling([Math]::Max($frame.Top, $panel.Top)),
        [Math]::Floor([Math]::Min($frame.Right, $panel.Right)), [Math]::Floor([Math]::Min($frame.Bottom, $panel.Bottom)))
    [void][IO.Directory]::CreateDirectory($EvidencePath)
    [MimiPanelCrop]::AssertUnobscured($Controls, $crop)
    $sample = [MimiPointerSmoke+PixelPatch]::new($crop.X, $crop.Y, $crop.Width, $crop.Height)
    $pixels = $sample.Read()
    [MimiPanelCrop]::AssertUnobscured($Controls, $crop)
    if ($window.ToString() -cne [MimiPointerSmoke]::ReadBounds($Controls).ToString()) { throw 'Panel moved during capture.' }
    [MimiPanelCrop]::Save($Controls, $crop, $pixels, (Join-Path $EvidencePath "$locale.png"))

    $items = [ordered]@{ source = $Labels.source; target = $Labels.target; systemAudio = $Labels.systemAudio; microphone = $Labels.microphone;
        mode0 = $Labels.modes[0]; mode1 = $Labels.modes[1]; mode2 = $Labels.modes[2]; moreSettings = $Labels.moreSettings }
    $controlsMetadata = [ordered]@{}
    $previousBounds = @()
    $selectedModes = 0
    foreach ($key in $items.Keys) {
        $named = [System.Windows.Automation.PropertyCondition]::new([System.Windows.Automation.AutomationElement]::NameProperty, [string]$items[$key])
        $isPicker = $key -in @('source', 'target')
        $roles = if ($isPicker) { [System.Windows.Automation.ControlType]::ComboBox }
                 elseif ($key -eq 'moreSettings') { [System.Windows.Automation.ControlType]::Button }
                 else { [System.Windows.Automation.ControlType]::Button, [System.Windows.Automation.ControlType]::CheckBox, [System.Windows.Automation.ControlType]::RadioButton }
        $matchesByRole = @($surface.FindAll($scope, $named) | Where-Object {
            -not $_.Current.IsOffscreen -and $roles -contains $_.Current.ControlType
        })
        if ($matchesByRole.Count -ne 1) { throw "$locale has $($matchesByRole.Count) visible $key controls." }
        $element = $matchesByRole[0]
        $box = $element.Current.BoundingRectangle
        Assert-PanelBounds $box $panel $key
        if ($box.Width -lt 24 -or $box.Height -lt 24) { throw "The $key target is smaller than 24 physical pixels." }
        foreach ($earlier in $previousBounds) {
            $intersection = [System.Windows.Rect]::Intersect($box, $earlier)
            if ($intersection.Width -gt 1 -and $intersection.Height -gt 1) { throw "The $key target overlaps another control." }
        }
        $previousBounds += $box
        $entry = [ordered]@{ bounds = @($box.Left, $box.Top, $box.Width, $box.Height); enabled = $element.Current.IsEnabled }
        if ($isPicker) {
            $expected = if ($key -eq 'source') { $Labels.sourceValue } else { $Labels.targetValue }
            $entry.valueEvidence = Get-PanelPickerValueEvidence $element $expected $key
        } elseif ($key -ne 'moreSettings') {
            $checked = Get-PanelToggleState $element $key
            $entry.selected = $checked
            if ($key -eq 'systemAudio' -and -not $checked -or $key -eq 'microphone' -and $checked) { throw 'Unexpected fixture audio selection.' }
            if ($key.StartsWith('mode') -and $checked) { $selectedModes++ }
        }
        $controlsMetadata[$key] = $entry
    }
    if ($selectedModes -ne 1) { throw 'The subtitle mode group must have one selection.' }
    $metadata = [ordered]@{ locale = $locale; window = @($frame.Left, $frame.Top, $frame.Width, $frame.Height);
        crop = @($crop.X, $crop.Y, $crop.Width, $crop.Height); controls = $controlsMetadata }
    [IO.File]::WriteAllText((Join-Path $EvidencePath "$locale.json"), ($metadata | ConvertTo-Json -Depth 5), [Text.UTF8Encoding]::new($false))
    Write-Output "Captured $locale panel ($($crop.Width)x$($crop.Height)); 8 control bounds, audio and subtitle selections verified."
}
