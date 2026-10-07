import java.util.Properties

plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
}

val appVersion = Properties().apply {
    rootProject.file("version.properties").inputStream().use { load(it) }
}
val releaseVersionName = requireNotNull(appVersion.getProperty("versionName"))
require(releaseVersionName.matches(Regex("[0-9]+\\.[0-9]+\\.[0-9]+"))) {
    "version.properties must contain a numeric versionName (major.minor.patch)"
}
val releaseVersionCode = requireNotNull(appVersion.getProperty("versionCode")?.toIntOrNull())
require(releaseVersionCode in 1..2100000000) {
    "version.properties must contain a positive Android versionCode"
}

val sharedCoreConfig = Properties().apply {
    rootProject.file("shared-core.properties").inputStream().use { load(it) }
}
val sharedCoreNdkVersion = requireNotNull(sharedCoreConfig.getProperty("ndkVersion"))
val sharedCoreMinSdk = requireNotNull(sharedCoreConfig.getProperty("minSdk").toIntOrNull())
val sharedCoreAbis = requireNotNull(sharedCoreConfig.getProperty("abis")).split(",")
require(sharedCoreAbis.toSet() == setOf("arm64-v8a", "armeabi-v7a", "x86_64", "x86"))

val repositoryRoot = rootProject.layout.projectDirectory.dir("..").asFile
val pythonExecutable = providers.environmentVariable("MIMI_PYTHON").orElse(
    if (System.getProperty("os.name").startsWith("Windows")) "python" else "python3"
)
val sharedCoreTarget = repositoryRoot.resolve("shared/target")
val sharedCoreHostOutput = layout.buildDirectory.dir("generated/shared-core/host")
val sharedCoreNativeInputs = files(
    fileTree(repositoryRoot.resolve("shared/mimi-core")) { include("Cargo.toml", "Cargo.lock", "src/**") },
    fileTree(repositoryRoot.resolve("shared/mimi-android-jni")) { include("Cargo.toml", "Cargo.lock", "src/**") },
    repositoryRoot.resolve("scripts/build-shared-core.py"),
    repositoryRoot.resolve("scripts/verify-shared-core.py"),
    rootProject.file("shared-core.properties"),
)
val rustVersion = providers.exec { commandLine("rustc", "-vV") }.standardOutput.asText

val buildSharedCoreHost = tasks.register<Exec>("buildSharedCoreHost") {
    workingDir(repositoryRoot)
    commandLine(pythonExecutable.get(), "scripts/build-shared-core.py", "--platform", "host",
        "--profile", "debug", "--target-dir", sharedCoreTarget.absolutePath,
        "--output", sharedCoreHostOutput.get().asFile.absolutePath)
    inputs.files(sharedCoreNativeInputs).withPathSensitivity(PathSensitivity.RELATIVE)
    inputs.property("rustVersion", rustVersion)
    outputs.dir(sharedCoreHostOutput)
}

android {
    namespace = "app.yuxino.mimi.android"
    compileSdk = 35
    buildToolsVersion = "35.0.0"
    ndkVersion = sharedCoreNdkVersion

    defaultConfig {
        applicationId = "app.yuxino.mimi.android"
        minSdk = sharedCoreMinSdk
        targetSdk = 35
        versionCode = releaseVersionCode
        versionName = releaseVersionName
        testInstrumentationRunner = "app.yuxino.mimi.android.UiSmokeInstrumentation"
        ndk { abiFilters += sharedCoreAbis }
    }

    buildTypes {
        release {
            isDebuggable = false
            isMinifyEnabled = false
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    sourceSets.getByName("test").resources.apply {
        srcDir(rootProject.file("../shared"))
        // Shared Rust sources and build caches are not JVM test resources.
        include("*.json")
    }

    sourceSets.getByName("main").assets.srcDir(rootProject.file("native-licenses"))

    packaging { jniLibs.useLegacyPackaging = false }

    kotlinOptions {
        jvmTarget = "17"
    }
}

val pinnedNdk = androidComponents.sdkComponents.sdkDirectory.map { it.dir("ndk/$sharedCoreNdkVersion") }
for (variant in listOf("debug", "release")) {
    val suffix = variant.replaceFirstChar { it.uppercaseChar() }
    val output = layout.buildDirectory.dir("generated/shared-core/$variant/jniLibs")
    val nativeBuild = tasks.register<Exec>("buildSharedCore$suffix") {
        workingDir(repositoryRoot)
        commandLine(pythonExecutable.get(), "scripts/build-shared-core.py", "--platform", "android",
            "--profile", variant, "--target-dir", sharedCoreTarget.absolutePath,
            "--ndk", pinnedNdk.get().asFile.absolutePath, "--output", output.get().asFile.absolutePath)
        inputs.files(sharedCoreNativeInputs).withPathSensitivity(PathSensitivity.RELATIVE)
        inputs.property("rustVersion", rustVersion)
        inputs.property("ndkVersion", sharedCoreNdkVersion)
        outputs.dir(output)
    }
    android.sourceSets.getByName(variant).jniLibs.srcDir(output)
    tasks.matching { it.name == "merge${suffix}JniLibFolders" }.configureEach { dependsOn(nativeBuild) }

    val apkName = if (variant == "release") "app-release-unsigned.apk" else "app-debug.apk"
    val apk = layout.buildDirectory.file("outputs/apk/$variant/$apkName")
    val verifyApk = tasks.register<Exec>("verifySharedCore${suffix}Apk") {
        dependsOn("package$suffix")
        workingDir(repositoryRoot)
        commandLine(pythonExecutable.get(), "scripts/verify-shared-core.py", "--apk", apk.get().asFile.absolutePath)
        inputs.file(apk)
        inputs.file(repositoryRoot.resolve("scripts/verify-shared-core.py"))
        // Verification is intentionally run after every requested assembly.
        outputs.upToDateWhen { false }
    }
    tasks.matching { it.name == "assemble$suffix" }.configureEach { dependsOn(verifyApk) }
}

tasks.withType<Test>().configureEach {
    dependsOn(buildSharedCoreHost)
    inputs.dir(sharedCoreHostOutput).withPathSensitivity(PathSensitivity.RELATIVE)
    systemProperty("java.library.path", sharedCoreHostOutput.get().asFile.absolutePath)
    systemProperty("mimi.repositoryRoot", repositoryRoot.absolutePath)
    inputs.file(repositoryRoot.resolve("src/lib/i18n.ts"))
}

dependencies {
    implementation("androidx.core:core-ktx:1.13.1")
    implementation("androidx.appcompat:appcompat:1.7.0")
    implementation("com.google.android.material:material:1.12.0")
    implementation("androidx.constraintlayout:constraintlayout:2.1.4")
    implementation("androidx.security:security-crypto:1.1.0-alpha06")
    implementation("com.squareup.okhttp3:okhttp:4.12.0")
    testImplementation("junit:junit:4.13.2")
    testImplementation("org.json:json:20240303")
}
