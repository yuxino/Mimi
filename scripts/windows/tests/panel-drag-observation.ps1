# Optional native trace/video for this isolated hosted acceptance only.
Add-Type -Path (Join-Path $PSScriptRoot 'PanelDragObservation.cs')
function Start-PanelDragObservation([IntPtr]$Root,[IntPtr]$Control,[string]$Directory) {
    $observer=[MimiPanelDragObservation]::new($Root,$Control)
    $record=@{Observer=$observer;Directory=$Directory;Metadata=@{frequency=$observer.Frequency;requestedFps=60;audio=$false;neutralBackground=$true;contentScope='offline root/control plus opaque QA background'}}
    try {
        if (-not $env:MIMI_QA_FFMPEG -or -not (Test-Path -LiteralPath $env:MIMI_QA_FFMPEG -PathType Leaf)) { throw 'Verified QA ffmpeg executable is required.' }
        $r=$observer.Region;$record.Metadata.region=@($r.Left,$r.Top,$r.Width,$r.Height)
        $path=Join-Path ([IO.Path]::GetTempPath()) ('mimi-panel-native-'+[guid]::NewGuid().ToString('N')+'.mp4');$record.Video=$path;$progress=Join-Path $Directory 'capture-progress.txt'
        $encoderArguments=@('-hide_banner','-loglevel','warning','-y','-f','gdigrab','-framerate','60','-draw_mouse','1',
            '-offset_x',[string]$r.Left,'-offset_y',[string]$r.Top,'-video_size',("{0}x{1}" -f $r.Width,$r.Height),'-i','desktop',
            '-an','-c:v','libx264','-preset','ultrafast','-tune','zerolatency','-bf','0','-crf','18','-pix_fmt','yuv420p',
            '-fps_mode','passthrough','-enc_time_base','1:1000000','-video_track_timescale','1000000','-movflags','+faststart','-stats_period','0.1','-progress',$progress,$path)
        $start=[Diagnostics.ProcessStartInfo]::new($env:MIMI_QA_FFMPEG)
        $start.UseShellExecute=$false;$start.CreateNoWindow=$true;$start.RedirectStandardInput=$true;$start.RedirectStandardError=$true
        $start.Arguments=($encoderArguments | ForEach-Object { '"'+$_+'"' }) -join ' '
        $record.Metadata.commandStartedHostTicks=[Diagnostics.Stopwatch]::GetTimestamp()
        $record.Process=[Diagnostics.Process]::Start($start);$record.Video=$path;$record.Progress=$progress
        $ready=[Diagnostics.Stopwatch]::StartNew()
        do {
            if ($record.Process.HasExited) { throw ('Native video recorder exited before a frame: '+$record.Process.StandardError.ReadToEnd()) }
            if (Test-Path -LiteralPath $progress) {
                $text=Read-SmokeMarker $progress
                $frame=@([regex]::Matches($text,'(?m)^frame=(\d+)\s*$'))
                if ($frame.Count -and [int]$frame[-1].Groups[1].Value -gt 0) {
                    $record.Metadata.firstFrameProgressObservedHostTicks=[Diagnostics.Stopwatch]::GetTimestamp()
                    $record.Metadata.firstProgress=$text
                    $record.Metadata.clockLimit='Progress is an observed encoder boundary; command start is not claimed as first capture time. Frames retain actual video PTS.'
                    $observer.AssertPrivateRegion();$observer.Mark('video-ready');return $record
                }
            }
            Start-Sleep -Milliseconds 20
        } until ($ready.ElapsedMilliseconds -ge 5000)
        throw 'Native video recorder did not report an encoded frame within 5000ms.'
    } catch {
        if ($record.Process -and -not $record.Process.HasExited) { $record.Process.Kill();$record.Process.WaitForExit(2000)|Out-Null }
        $record.Metadata.setupFailure=[string]$_
        [IO.File]::WriteAllText((Join-Path $Directory 'native-events.json'),(@{frequency=$observer.Frequency;samples=$observer.Snapshot();dropped=$observer.DroppedSamples}|ConvertTo-Json -Depth 8),[Text.UTF8Encoding]::new($false))
        [IO.File]::WriteAllText((Join-Path $Directory 'capture-metadata.json'),($record.Metadata|ConvertTo-Json -Depth 8),[Text.UTF8Encoding]::new($false))
        if($record.Video -and (Test-Path -LiteralPath $record.Video)){Remove-Item -LiteralPath $record.Video -Force}
        $observer.Dispose();throw
    }
}
function Stop-PanelDragVideo($Record) {
    if($Record.Metadata.videoStopped){return}
    $observer=$Record.Observer;$process=$Record.Process;$privacyFailure=$null
    try {
        $observer.Mark('video-stop-requested')
        try{$observer.AssertPrivateRegion()}catch{$privacyFailure=$_}
        # Finalize even after a failed guard: never leave desktop capture running.
        if (-not $process.HasExited) { $process.StandardInput.WriteLine('q');$process.StandardInput.Flush() }
        if (-not $process.WaitForExit(5000)) { $process.Kill();throw 'Native video did not finalize within 5000ms.' }
        $stderr=$process.StandardError.ReadToEnd()
        [IO.File]::WriteAllText((Join-Path $Record.Directory 'capture-stderr.log'),$stderr,[Text.UTF8Encoding]::new($false))
        if ($process.ExitCode -ne 0) { throw 'Native video recorder failed.' }
        $Record.Metadata.completedHostTicks=[Diagnostics.Stopwatch]::GetTimestamp()
        if($privacyFailure){
            $Record.Metadata.videoScopeValid=$false
            Remove-Item -LiteralPath $Record.Video -Force
            throw $privacyFailure
        }
        $Record.Metadata.videoScopeValid=$true
        $probe=Join-Path ([IO.Path]::GetDirectoryName($env:MIMI_QA_FFMPEG)) 'ffprobe.exe'
        $info=& $probe -v error -select_streams v:0 -show_entries 'stream=nb_frames,avg_frame_rate,time_base,width,height:frame=best_effort_timestamp_time,pkt_duration_time' -of json $Record.Video
        if ($LASTEXITCODE -ne 0) { throw 'Recorded video PTS inspection failed.' }
        [IO.File]::WriteAllText((Join-Path $Record.Directory 'capture-frames.json'),($info -join "`n"),[Text.UTF8Encoding]::new($false))
        $decoded=($info -join "`n")|ConvertFrom-Json
        $pts=@($decoded.frames | ForEach-Object {[double]::Parse($_.best_effort_timestamp_time,[Globalization.CultureInfo]::InvariantCulture)})
        if ($pts.Count -lt 2) { throw 'Native recording contains fewer than two decoded frames.' }
        $gaps=@();for($i=1;$i -lt $pts.Count;$i++){if($pts[$i] -le $pts[$i-1]){throw 'Recorded display timestamps are not increasing.'};$gaps+=($pts[$i]-$pts[$i-1])}
        $Record.Metadata.decodedFrames=$pts.Count;$Record.Metadata.firstPts=$pts[0];$Record.Metadata.lastPts=$pts[-1]
        $Record.Metadata.measuredFps=($pts.Count-1)/($pts[-1]-$pts[0]);$Record.Metadata.maxFrameGapSeconds=($gaps|Measure-Object -Maximum).Maximum
        $Record.Metadata.finalProgress=Read-SmokeMarker $Record.Progress
        Move-Item -LiteralPath $Record.Video -Destination (Join-Path $Record.Directory 'capture.mp4')
        $Record.Metadata.videoAccepted=$true
    } finally {
        if(-not $process.HasExited){$process.Kill();$null=$process.WaitForExit(2000)}
        $Record.Metadata.videoStopped=$true
        $process.Dispose()
        if(-not $Record.Metadata.videoAccepted -and (Test-Path -LiteralPath $Record.Video)){Remove-Item -LiteralPath $Record.Video -Force}
    }
}
function Stop-PanelDragObservation($Record) {
    $observer=$Record.Observer
    try { Stop-PanelDragVideo $Record }
    finally {
        $Record.Metadata.droppedObserverSamples=$observer.DroppedSamples
        [IO.File]::WriteAllText((Join-Path $Record.Directory 'native-events.json'),(@{frequency=$observer.Frequency;samples=$observer.Snapshot();dropped=$observer.DroppedSamples}|ConvertTo-Json -Depth 8),[Text.UTF8Encoding]::new($false))
        [IO.File]::WriteAllText((Join-Path $Record.Directory 'capture-metadata.json'),($Record.Metadata|ConvertTo-Json -Depth 8),[Text.UTF8Encoding]::new($false))
        $observer.Dispose()
    }
}
