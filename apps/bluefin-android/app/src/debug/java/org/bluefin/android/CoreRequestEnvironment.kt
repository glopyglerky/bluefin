package org.bluefin.android

import org.json.JSONObject

/** Instrumentation-only loopback HTTP opt-in; no UI, Intent, preference or account injection. */
internal object CoreRequestEnvironment {
    @Volatile var loopbackFixture = false
    fun decorate(input: JSONObject) {
        if (loopbackFixture && input.optString("server") == "http://127.0.0.1:18765") {
            input.put("allowLocalHttp", true)
        }
    }
}
