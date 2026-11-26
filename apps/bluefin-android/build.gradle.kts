plugins {
    id("com.android.application") version "8.10.1" apply false
    id("org.jetbrains.kotlin.android") version "2.1.20" apply false
    id("org.jetbrains.kotlin.plugin.compose") version "2.1.20" apply false
}
// Keep generated output outside the source checkout. Supply -PbluefinBuildDir.
allprojects {
    val outputRoot = providers.gradleProperty("bluefinBuildDir").orNull
        ?: error("Pass -PbluefinBuildDir pointing to bounded generated storage")
    layout.buildDirectory.set(file(outputRoot + "/" + project.name))
}
