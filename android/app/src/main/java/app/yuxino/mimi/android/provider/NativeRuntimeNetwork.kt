package app.yuxino.mimi.android.provider

import android.security.NetworkSecurityPolicy
import org.json.JSONArray
import org.json.JSONObject
import java.net.InetSocketAddress
import java.net.Proxy
import java.net.ProxySelector
import java.net.URI
import java.security.KeyStore
import java.util.Base64
import javax.net.ssl.TrustManagerFactory
import javax.net.ssl.X509TrustManager

/** Read platform routing and trust once. No connections, credentials or fallback trust stores. */
internal object NativeRuntimeNetwork {
    fun textSnapshot(configuration: JSONObject): JSONObject {
        val endpoints = NativeRuntimeConfiguration.exchange(JSONObject().put("operation", "text_probe_endpoint").put("configuration", configuration))
        val proxy = nativeProxySnapshot(JSONObject().put("mode", "system").put("url", JSONObject.NULL),
            endpoints.getString("textEndpoint"), ProxySelector.getDefault()) {
            NetworkSecurityPolicy.getInstance().isCleartextTrafficPermitted(it)
        }
        return JSONObject().put("speechProxy", JSONObject().put("mode", "direct").put("url", JSONObject.NULL))
            .put("textProxy", proxy).put("trustRoots", trustRoots())
    }

    fun snapshot(configuration: JSONObject): JSONObject {
        val endpoints = NativeRuntimeConfiguration.endpoints(configuration)
        val speech = endpoints.getString("speechEndpoint")
        val text = endpoints.optString("textEndpoint").takeIf { it.isNotEmpty() && it != "null" }
        val cleartextPolicy: (String) -> Boolean = { NetworkSecurityPolicy.getInstance().isCleartextTrafficPermitted(it) }
        val selector = ProxySelector.getDefault()
        val speechProxy = nativeProxySnapshot(configuration.getJSONObject("networkProxy"), speech, selector, cleartextPolicy)
        val textProxy = nativeOptionalTextProxySnapshot(configuration.getJSONObject("textNetworkProxy"), text, selector, cleartextPolicy)
        return JSONObject().put("speechProxy", speechProxy).put("textProxy", textProxy).put("trustRoots", trustRoots())
    }

    private fun trustRoots(): JSONArray = try {
        val factory = TrustManagerFactory.getInstance(TrustManagerFactory.getDefaultAlgorithm())
        factory.init(null as KeyStore?)
        val trust = factory.trustManagers.filterIsInstance<X509TrustManager>().singleOrNull()
            ?: throw IllegalArgumentException("network_trust_unavailable")
        val certificates = trust.acceptedIssuers
        require(certificates.isNotEmpty()) { "network_trust_unavailable" }
        JSONArray(certificates.map { Base64.getEncoder().encodeToString(it.encoded) })
    } catch (_: Exception) {
        throw IllegalArgumentException("network_trust_unavailable")
    }
}

/** An inactive text stage has no route to resolve or connection to open. */
internal fun nativeOptionalTextProxySnapshot(
    choice: JSONObject,
    endpoint: String?,
    selector: ProxySelector?,
    cleartextPermitted: (String) -> Boolean,
): JSONObject = endpoint?.let { nativeProxySnapshot(choice, it, selector, cleartextPermitted) }
    ?: JSONObject().put("mode", "direct").put("url", JSONObject.NULL)

/** The routing query uses HTTP semantics; the actual WebSocket endpoint is never rewritten. */
internal fun nativeProxySnapshot(
    choice: JSONObject,
    endpoint: String,
    selector: ProxySelector?,
    cleartextPermitted: (String) -> Boolean,
): JSONObject {
    val destination = try { URI(endpoint) } catch (_: Exception) { throw IllegalArgumentException("network_endpoint_invalid") }
    val host = destination.host ?: throw IllegalArgumentException("network_endpoint_invalid")
    require(destination.rawUserInfo == null) { "network_endpoint_invalid" }
    val scheme = destination.scheme?.lowercase()
    require(scheme in listOf("https", "wss", "http", "ws")) { "network_endpoint_invalid" }
    if (scheme == "http" || scheme == "ws") {
        require(cleartextPermitted(host.removePrefix("[").removeSuffix("]"))) { "network_cleartext_blocked" }
    }
    fun direct() = JSONObject().put("mode", "direct").put("url", JSONObject.NULL)
    fun custom(url: String): JSONObject {
        val parsed = try { URI(url) } catch (_: Exception) { throw IllegalArgumentException("network_proxy_invalid_url") }
        require(parsed.scheme in listOf("http", "socks5", "socks5h") && parsed.host != null &&
            parsed.rawUserInfo == null && parsed.rawQuery == null && parsed.rawFragment == null &&
            parsed.rawPath in listOf("", "/") && parsed.port != 0 && parsed.port <= 65535) { "network_proxy_invalid_url" }
        return JSONObject().put("mode", "custom").put("url", url)
    }
    return when (choice.getString("mode")) {
        "direct" -> direct()
        "custom" -> custom(choice.getString("url"))
        "system" -> {
            val query = if (scheme == "wss" || scheme == "ws")
                URI((if (scheme == "wss") "https" else "http") + endpoint.substring(endpoint.indexOf(':'))) else destination
            val selected = try { selector?.select(query)?.firstOrNull() }
                catch (_: Exception) { throw IllegalArgumentException("network_proxy_unavailable") }
                ?: throw IllegalArgumentException("network_proxy_unavailable")
            when (selected.type()) {
                Proxy.Type.DIRECT -> direct()
                Proxy.Type.HTTP -> {
                    val address = selected.address() as? InetSocketAddress ?: throw IllegalArgumentException("network_proxy_invalid_url")
                    require(address.port in 1..65535) { "network_proxy_invalid_url" }
                    // hostString does not trigger reverse DNS or a network request.
                    custom(URI("http", null, address.hostString, address.port, null, null, null).toString())
                }
                // Java's SOCKS value does not specify protocol version or DNS policy.
                Proxy.Type.SOCKS -> throw IllegalArgumentException("network_proxy_unsupported_scheme")
                else -> throw IllegalArgumentException("network_proxy_unsupported_scheme")
            }
        }
        else -> throw IllegalArgumentException("network_proxy_invalid_mode")
    }
}
