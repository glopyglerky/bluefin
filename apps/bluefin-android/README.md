# Bluefin Android

Native phone, tablet and TV client candidate. See [Android implementation and remaining gates](../../docs/bluefin/android.md).

The Gradle application can run the original synthetic demo without JNI. Live operations require the packaged JNI library and qualified Android support from the existing shared Rust core. This candidate has Kotlin source-check evidence, but no APK, Android native build or device acceptance.

Generated output belongs in bounded build storage. Pass `-PbluefinBuildDir` and `-PbluefinJniDir`, and keep Gradle user/project caches outside the checkout. Do not put credentials or deployment settings into this tree.
