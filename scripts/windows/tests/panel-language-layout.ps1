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
    [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr window);

    public static void AssertUnobscured(IntPtr target, Rectangle crop) {
        for (var window = GetWindow(target, 3); window != IntPtr.Zero; window = GetWindow(window, 3)) { // GW_HWNDPREV
            Rect rect; int cloaked;
            if (IsWindowVisible(window) && GetWindowRect(window, out rect) && rect.Right > rect.Left && rect.Bottom > rect.Top
                && !(DwmGetWindowAttribute(window, 14, out cloaked, 4) == 0 && cloaked != 0)
                && crop.IntersectsWith(Rectangle.FromLTRB(rect.Left, rect.Top, rect.Right, rect.Bottom)))
                throw new InvalidOperationException("Another visible window overlaps the panel capture.");
        }
    }

    public static void Save(Rectangle crop, int[] pixels, uint dpi, string path) {
        if (pixels.Length != crop.Width * crop.Height) throw new InvalidOperationException("Incomplete panel pixels.");
        // The WebView is transparent outside its 13-CSS-pixel rounded panel.
        // Discard these corner pixels, including the mixed antialias edge.
        int radius = (int)Math.Ceiling(13.0 * dpi / 96.0);
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

function Get-PanelPickerValueEvidence($Element, [string]$Key) {
    $scope = [System.Windows.Automation.TreeScope]::Descendants
    $text = [System.Windows.Automation.PropertyCondition]::new(
        [System.Windows.Automation.AutomationElement]::ControlTypeProperty, [System.Windows.Automation.ControlType]::Text)
    $children = @($Element.FindAll($scope, $text) | Where-Object { -not $_.Current.IsOffscreen -and -not [string]::IsNullOrWhiteSpace($_.Current.Name) })
    if ($children.Count) {
        $value = $children[0].Current.Name
        foreach ($child in $children) {
            if ($child.Current.Name -cne $value) { throw "The closed $Key picker contains conflicting text values." }
            Assert-PanelBounds $child.Current.BoundingRectangle $Element.Current.BoundingRectangle "$Key text"
        }
        $box = $children[0].Current.BoundingRectangle
        return @{ value = $value; evidence = 'text bounds'; bounds = @($box.Left, $box.Top, $box.Width, $box.Height) }
    }
    $pattern = $null
    if ($Element.TryGetCurrentPattern([System.Windows.Automation.ValuePattern]::Pattern, [ref]$pattern) -and
        -not [string]::IsNullOrWhiteSpace($pattern.Current.Value)) {
        return @{ value = $pattern.Current.Value; evidence = 'accessible value only' }
    }
    if ($Element.TryGetCurrentPattern([System.Windows.Automation.SelectionPattern]::Pattern, [ref]$pattern)) {
        $selected = $pattern.GetCurrentSelection()
        if ($selected.Count -eq 1 -and -not [string]::IsNullOrWhiteSpace($selected[0].Current.Name)) {
            return @{ value = $selected[0].Current.Name; evidence = 'selected item only' }
        }
    }
    throw "The closed $Key picker did not expose a readable selected value."
}

function Save-PanelLanguageScreenshot([IntPtr]$Controls, [string]$EvidencePath, [string]$FileName, [switch]$PassThru) {
    if ($FileName -cnotmatch '^[A-Za-z0-9][A-Za-z0-9_-]*\.png$') { throw 'Invalid panel screenshot filename.' }
    $window = [MimiPointerSmoke]::ReadBounds($Controls)
    $dpi = [MimiPanelCrop]::GetDpiForWindow($Controls)
    if ($dpi -eq 0) { throw 'Could not determine the panel DPI.' }
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
    [MimiPanelCrop]::Save($crop, $pixels, $dpi, (Join-Path $EvidencePath $FileName))
    if ($PassThru) { return @{ dpi = $dpi; frame = $frame; panel = $panel; crop = $crop; surface = $surface } }
}

function Save-PanelLanguageEvidence([IntPtr]$Controls, $Labels, [string]$EvidencePath) {
    $locale = [string]$Labels.locale
    if (@('zh', 'zh-TW', 'en', 'ja', 'ko', 'de', 'fr', 'th', 'ru') -cnotcontains $locale -or @($Labels.modes).Count -ne 3) {
        throw 'Invalid panel language fixture.'
    }
    $capture = Save-PanelLanguageScreenshot $Controls $EvidencePath "$locale.png" -PassThru
    $dpi = $capture.dpi
    $scale = $dpi / 96.0
    $minimumTarget = [Math]::Floor(24 * $scale)
    $frame = $capture.frame
    $panel = $capture.panel
    $crop = $capture.crop
    if ([Math]::Abs($panel.Width - 280 * $scale) -gt 1 -or [Math]::Abs($frame.Width - 280 * $scale) -gt 1) {
        throw "Expected a 280-logical-pixel panel at $dpi DPI (window=$($frame.Width), dialog=$($panel.Width) physical pixels)."
    }

    $items = [ordered]@{ source = $Labels.source; target = $Labels.target; systemAudio = $Labels.systemAudio; microphone = $Labels.microphone;
        mode0 = $Labels.modes[0]; mode1 = $Labels.modes[1]; mode2 = $Labels.modes[2]; moreSettings = $Labels.moreSettings }
    $controlsMetadata = [ordered]@{}
    $pickerValues = @{}
    $previousBounds = @()
    $scope = [System.Windows.Automation.TreeScope]::Descendants
    foreach ($key in $items.Keys) {
        $named = [System.Windows.Automation.PropertyCondition]::new([System.Windows.Automation.AutomationElement]::NameProperty, [string]$items[$key])
        $isPicker = $key -in @('source', 'target')
        $roles = if ($isPicker) { [System.Windows.Automation.ControlType]::ComboBox }
                 elseif ($key -eq 'moreSettings') { [System.Windows.Automation.ControlType]::Button }
                 else { [System.Windows.Automation.ControlType]::Button, [System.Windows.Automation.ControlType]::CheckBox, [System.Windows.Automation.ControlType]::RadioButton }
        $matchesByRole = @($capture.surface.FindAll($scope, $named) | Where-Object {
            -not $_.Current.IsOffscreen -and $roles -contains $_.Current.ControlType
        })
        if ($matchesByRole.Count -ne 1) { throw "$locale has $($matchesByRole.Count) visible $key controls." }
        $element = $matchesByRole[0]
        $box = $element.Current.BoundingRectangle
        Assert-PanelBounds $box $panel $key
        if ($box.Width -lt $minimumTarget -or $box.Height -lt $minimumTarget) { throw "The $key target is smaller than 24 logical pixels at $dpi DPI." }
        foreach ($earlier in $previousBounds) {
            $intersection = [System.Windows.Rect]::Intersect($box, $earlier)
            if ($intersection.Width -gt 1 -and $intersection.Height -gt 1) { throw "The $key target overlaps another control." }
        }
        $previousBounds += $box
        $entry = [ordered]@{ bounds = @($box.Left, $box.Top, $box.Width, $box.Height); enabled = $element.Current.IsEnabled }
        if ($isPicker) {
            $reading = Get-PanelPickerValueEvidence $element $key
            $pickerValues[$key] = $reading.value
            $entry.value = $reading.value
            $entry.valueEvidence = $reading.evidence
            if ($reading.ContainsKey('bounds')) { $entry.valueBounds = $reading.bounds }
        } elseif ($key -ne 'moreSettings') {
            $checked = Get-PanelToggleState $element $key
            $entry.selected = $checked
            if ($key -eq 'systemAudio' -and -not $checked -or $key -eq 'microphone' -and $checked) { throw 'Unexpected fixture audio selection.' }
            if ($key.StartsWith('mode') -and $checked -ne ($key -eq 'mode1')) { throw 'Only the bilingual subtitle mode should be selected.' }
        }
        $controlsMetadata[$key] = $entry
    }
    $metadata = [ordered]@{ locale = $locale; dpi = $dpi; window = @($frame.Left, $frame.Top, $frame.Width, $frame.Height);
        crop = @($crop.X, $crop.Y, $crop.Width, $crop.Height); controls = $controlsMetadata }
    [IO.File]::WriteAllText((Join-Path $EvidencePath "$locale.json"), ($metadata | ConvertTo-Json -Depth 5), [Text.UTF8Encoding]::new($false))
    Write-Host "Captured $locale panel ($($crop.Width)x$($crop.Height) at $dpi DPI); 8 control bounds, audio and bilingual selections verified."
    return $pickerValues
}
