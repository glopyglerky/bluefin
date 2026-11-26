package org.bluefin.android

import org.json.JSONObject

/** Release requests always use production Core URL and authentication validation. */
internal object CoreRequestEnvironment {
    fun decorate(input: JSONObject) = Unit
}
