package app.yuxino.mimi.android.capture

import android.Manifest
import android.media.AudioAttributes
import android.media.AudioFormat
import android.media.AudioPlaybackCaptureConfiguration
import android.media.AudioRecord
import android.media.MediaRecorder
import android.media.projection.MediaProjection
import android.os.Process
import android.os.SystemClock
import android.util.Log
import androidx.annotation.RequiresPermission
import app.yuxino.mimi.android.provider.ProviderEngine
import app.yuxino.mimi.android.resample.StreamResampler
import java.util.concurrent.atomic.AtomicBoolean
import kotlin.concurrent.thread

/**
 * Owns one native input's recorder, audio worker and capture observations.
 * The service prepares every selected lane before starting any worker, and
 * stops every recorder before releasing a lane or finishing its shared runtime.
 */
internal class CaptureLane(val source: CaptureSource, val engine: ProviderEngine) {
    private var record: AudioRecord? = null
    private var worker: Thread? = null
    private val capturing = AtomicBoolean(false)
    private var health: CaptureHealth? = null

    val isCapturing: Boolean get() = capturing.get()

    fun observation(nowMs: Long): CaptureHealth.Snapshot? = health?.snapshot(nowMs)

    @RequiresPermission(Manifest.permission.RECORD_AUDIO)
    fun prepare(projection: MediaProjection?) {
        val channels = if (source == CaptureSource.SYSTEM) {
            AudioFormat.CHANNEL_IN_STEREO
        } else {
            AudioFormat.CHANNEL_IN_MONO
        }
        val encoding = if (source == CaptureSource.SYSTEM) {
            AudioFormat.ENCODING_PCM_FLOAT
        } else {
            AudioFormat.ENCODING_PCM_16BIT
        }
        val format = AudioFormat.Builder()
            .setEncoding(encoding)
            .setSampleRate(CAPTURE_RATE_HZ)
            .setChannelMask(channels)
            .build()
        val minBuffer = AudioRecord.getMinBufferSize(CAPTURE_RATE_HZ, channels, encoding)
        check(minBuffer > 0) { "unsupported_capture_format" }
        val builder = AudioRecord.Builder()
            .setAudioFormat(format)
            .setBufferSizeInBytes(minBuffer * 4)
        if (source == CaptureSource.SYSTEM) {
            builder.setAudioPlaybackCaptureConfig(
                AudioPlaybackCaptureConfiguration.Builder(checkNotNull(projection))
                    .addMatchingUsage(AudioAttributes.USAGE_MEDIA)
                    .addMatchingUsage(AudioAttributes.USAGE_GAME)
                    .addMatchingUsage(AudioAttributes.USAGE_UNKNOWN)
                    .build(),
            )
        } else {
            builder.setAudioSource(MediaRecorder.AudioSource.MIC)
        }
        record = builder.build()
        check(record?.state == AudioRecord.STATE_INITIALIZED) { "capture_uninitialized" }
    }

    fun start(onAudioSubmitted: (ByteArray) -> Unit, onFailure: () -> Unit) {
        val record = checkNotNull(record)
        val resampler = StreamResampler(CAPTURE_RATE_HZ, engine.sampleRateHz, source.channels)
        val floatBuffer = if (source == CaptureSource.SYSTEM) {
            FloatArray(CAPTURE_RATE_HZ / 10 * source.channels)
        } else null
        val shortBuffer = if (source == CaptureSource.MICROPHONE) {
            ShortArray(CAPTURE_RATE_HZ / 10)
        } else null
        val sessionHealth = CaptureHealth(SystemClock.elapsedRealtime())
        health = sessionHealth
        record.startRecording()
        check(record.recordingState == AudioRecord.RECORDSTATE_RECORDING) { "capture_not_started" }
        capturing.set(true)
        worker = thread(name = "mimi-capture-${source.id}") {
            runCaptureWorker(capturing, onFailure, release = { record.release() }) {
                Process.setThreadPriority(Process.THREAD_PRIORITY_URGENT_AUDIO)
                while (capturing.get()) {
                    val read = if (floatBuffer != null) {
                        record.read(floatBuffer, 0, floatBuffer.size, AudioRecord.READ_BLOCKING)
                    } else {
                        record.read(checkNotNull(shortBuffer), 0, shortBuffer.size, AudioRecord.READ_BLOCKING)
                    }
                    if (!capturing.get()) break
                    check(read >= 0) { "capture_read_failed" }
                    if (read == 0) continue
                    val samples = floatBuffer?.copyOf(read)
                        ?: FloatArray(read) { checkNotNull(shortBuffer)[it] / 32768f }
                    val pcm = resampler.push(samples)
                    sessionHealth.observe(pcm, SystemClock.elapsedRealtime())
                    if (capturing.get() && pcm.isNotEmpty()) {
                        engine.sendAudio(pcm)
                        onAudioSubmitted(pcm)
                    }
                }
            }
        }
    }

    /** Stop PCM admission before asking the runtime to drain or abort. */
    fun stopCapture() {
        capturing.set(false)
        try {
            record?.stop()
        } catch (_: Exception) { }
    }

    /** Also handles a lane whose recorder or worker failed during preparation. */
    fun release() {
        try {
            if (worker != null) {
                worker?.join(600)
            } else {
                record?.release()
            }
        } catch (_: Exception) { }
        try {
            engine.stop()
        } catch (_: Exception) {
            Log.w("MimiService", "capture_engine_stop_failed")
        }
    }

    private companion object {
        const val CAPTURE_RATE_HZ = 48_000
    }
}

internal fun runCaptureWorker(
    capturing: AtomicBoolean,
    onFailure: () -> Unit,
    release: () -> Unit,
    body: () -> Unit,
) {
    try {
        body()
    } catch (_: Exception) {
        if (capturing.get()) onFailure()
    } finally {
        release()
    }
}
