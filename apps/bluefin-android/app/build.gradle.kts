plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
    id("org.jetbrains.kotlin.plugin.compose")
}
android {
    namespace = "org.bluefin.android"
    compileSdk = 36
    defaultConfig {
        applicationId = "org.bluefin.android"
        minSdk = 26
        targetSdk = 36
        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
        versionCode = 1
        versionName = "0.1.0-prealpha"
        ndk { abiFilters += listOf("arm64-v8a", "armeabi-v7a", "x86_64") }
    }
    buildFeatures { compose = true }
    compileOptions { sourceCompatibility = JavaVersion.VERSION_17; targetCompatibility = JavaVersion.VERSION_17 }
    kotlinOptions { jvmTarget = "17" }
    packaging { jniLibs.useLegacyPackaging = false }
    sourceSets["main"].jniLibs.srcDir(providers.gradleProperty("bluefinJniDir").getOrElse("src/main/jniLibs"))
}
dependencies {
    implementation(platform("androidx.compose:compose-bom:2025.04.01"))
    implementation("androidx.activity:activity-compose:1.10.1")
    implementation("androidx.compose.material3:material3")
    implementation("androidx.compose.foundation:foundation")
    implementation("androidx.compose.ui:ui")
    implementation("androidx.lifecycle:lifecycle-viewmodel-compose:2.9.0")
    implementation("androidx.lifecycle:lifecycle-runtime-compose:2.9.0")
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-android:1.10.2")
    implementation("androidx.media3:media3-exoplayer:1.6.1")
    implementation("androidx.media3:media3-exoplayer-hls:1.6.1")
    implementation("androidx.media3:media3-ui:1.6.1")
    implementation("androidx.media3:media3-session:1.6.1")
    androidTestImplementation("androidx.test:runner:1.6.2")
    androidTestImplementation("androidx.test.ext:junit:1.2.1")
    testImplementation("junit:junit:4.13.2")
    testImplementation("org.json:json:20240303")
}

// Every packaged candidate must carry the portable core for every advertised ABI.
val verifyBluefinNative by tasks.registering {
    val nativeRoot = providers.gradleProperty("bluefinJniDir")
        .map { file(it) }.getOrElse(file("src/main/jniLibs"))
    inputs.dir(nativeRoot)
    doLast {
        listOf("arm64-v8a", "armeabi-v7a", "x86_64").forEach { abi ->
            val library = nativeRoot.resolve("$abi/libbluefin_android.so")
            check(library.isFile && library.length() > 0) {
                "Build the Android Rust core for $abi and supply -PbluefinJniDir"
            }
        }
    }
}
tasks.named("preBuild") { dependsOn(verifyBluefinNative) }

// One canonical legal resource tree; generated copies live in the external build directory.
val legalRoot = rootProject.file("../../docs/legal/bluefin-clients")
val legalAssets = layout.buildDirectory.dir("generated/legalAssets")
val verifyBluefinLegal by tasks.registering {
    doLast {
        val indexes = listOf("aarch64-linux-android", "armv7-linux-androideabi", "x86_64-linux-android")
        check(legalRoot.resolve("first-party/LICENSE").isFile) { "Missing Bluefin legal resources" }
        indexes.forEach { target ->
            val index = legalRoot.resolve("indexes/android-$target.json")
            check(index.isFile) { "Missing license index: $target" }
            val json = groovy.json.JsonSlurper().parse(index) as Map<*, *>
            val references = (json["first_party"] as List<*>) + listOf(json["standard_library_html"]) +
                (json["packages"] as List<*>).flatMap { (it as Map<*, *>)["texts"] as List<*> }
            check(references.isNotEmpty()) { "Empty license index: $target" }
            references.forEach { value ->
                val ref = value as String
                check(!ref.startsWith("/") && ref.none { it in ":\\%?#" }) { "Invalid legal reference" }
                val resource = index.parentFile.resolve(ref).canonicalFile
                check(resource.toPath().startsWith(legalRoot.canonicalFile.toPath()) && resource.isFile && resource.length() > 0) {
                    "Missing or invalid local legal resource: $ref"
                }
            }
        }
    }
}
val syncBluefinLegal by tasks.registering(Sync::class) {
    dependsOn(verifyBluefinLegal)
    from(legalRoot) { into("legal") }
    into(legalAssets)
}
android.sourceSets["main"].assets.srcDir(legalAssets)
tasks.named("preBuild") { dependsOn(syncBluefinLegal) }
