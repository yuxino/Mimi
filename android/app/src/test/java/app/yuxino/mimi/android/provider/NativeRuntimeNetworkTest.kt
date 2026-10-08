package app.yuxino.mimi.android.provider

import org.json.JSONObject
import org.junit.Assert.*
import org.junit.Test
import java.io.IOException
import java.net.InetSocketAddress
import java.net.Proxy
import java.net.ProxySelector
import java.net.SocketAddress
import java.net.URI

class NativeRuntimeNetworkTest {
    private fun choice(mode: String, url: String? = null) = JSONObject().put("mode", mode).put("url", url ?: JSONObject.NULL)
    private fun selector(block: (URI) -> List<Proxy>) = object : ProxySelector() {
        override fun select(uri: URI): List<Proxy> = block(uri)
        override fun connectFailed(uri: URI, sa: SocketAddress, ioe: IOException) = Unit
    }

    @Test fun systemSnapshotReadsTheActualDestinationAndPinsItsFirstRoute() {
        var queried: URI? = null
        val routing = selector { uri ->
            queried = uri
            listOf(Proxy(Proxy.Type.HTTP, InetSocketAddress.createUnresolved("proxy.example.invalid", 8080)), Proxy.NO_PROXY)
        }
        val snapshot = nativeProxySnapshot(choice("system"), "wss://speech.example.invalid/stream?model=synthetic", routing) { false }
        assertEquals("https", queried!!.scheme)
        assertEquals("speech.example.invalid", queried!!.host)
        assertEquals("/stream", queried!!.path)
        assertEquals("custom", snapshot.getString("mode"))
        assertEquals("http://proxy.example.invalid:8080", snapshot.getString("url"))
    }

    @Test fun systemResolutionFailureNeverTurnsIntoDirect() {
        for (routing in listOf(null, selector { emptyList() }, selector { throw IllegalStateException("synthetic") })) {
            val error = assertThrows(IllegalArgumentException::class.java) {
                nativeProxySnapshot(choice("system"), "https://text.example.invalid/v1", routing) { false }
            }
            assertEquals("network_proxy_unavailable", error.message)
        }
    }

    @Test fun directAndCustomDoNotQueryTheSystemSelector() {
        val unexpected = selector { fail("Explicit routing queried system settings"); emptyList() }
        assertEquals("direct", nativeProxySnapshot(choice("direct"), "https://speech.example.invalid", unexpected) { false }.getString("mode"))
        for (url in listOf("http://127.0.0.1:8080", "socks5h://proxy.example.invalid:1080")) {
            assertEquals(url, nativeProxySnapshot(choice("custom", url), "https://speech.example.invalid", unexpected) { false }.getString("url"))
        }
        assertThrows(IllegalArgumentException::class.java) {
            nativeProxySnapshot(choice("custom", "http://user:secret@proxy.example.invalid:8080"), "https://speech.example.invalid", unexpected) { false }
        }
    }

    @Test fun cleartextRequiresThePlatformPolicyAfterTheSavedOptInValidator() {
        var host: String? = null
        val allowed = nativeProxySnapshot(choice("direct"), "http://10.0.2.2:8000/v1", null) { requested -> host = requested; requested == "10.0.2.2" }
        assertEquals("10.0.2.2", host)
        assertEquals("direct", allowed.getString("mode"))
        assertThrows(IllegalArgumentException::class.java) {
            nativeProxySnapshot(choice("direct"), "http://10.0.2.2:8000/v1", null) { false }
        }
    }

    @Test fun unspecifiedSystemSocksProtocolFailsRatherThanInventingItsDnsPolicy() {
        val socks = selector { listOf(Proxy(Proxy.Type.SOCKS, InetSocketAddress.createUnresolved("proxy.example.invalid", 1080))) }
        val error = assertThrows(IllegalArgumentException::class.java) { nativeProxySnapshot(choice("system"), "https://speech.example.invalid", socks) { false } }
        assertEquals("network_proxy_unsupported_scheme", error.message)
    }

    @Test fun originalAndIntegratedSessionsDoNotResolveAnInactiveTextStage() {
        val unexpected = selector { fail("Inactive text stage queried system settings"); emptyList() }
        val snapshot = nativeOptionalTextProxySnapshot(choice("system"), null, unexpected) { fail("Inactive text stage checked cleartext"); false }
        assertEquals("direct", snapshot.getString("mode"))
        assertTrue(snapshot.isNull("url"))
        val error = assertThrows(IllegalArgumentException::class.java) {
            nativeOptionalTextProxySnapshot(choice("system"), "https://text.example.invalid/v1", null) { false }
        }
        assertEquals("network_proxy_unavailable", error.message)
    }
}
