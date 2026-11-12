import java.util.Properties
import org.gradle.api.artifacts.component.ModuleComponentIdentifier
import org.jetbrains.kotlin.gradle.tasks.KotlinCompile

plugins {
    id("com.android.application")
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
    fileTree(repositoryRoot.resolve("shared/mimi-core")) { include("Cargo.toml", "Cargo.lock", "build.rs", "src/**", ".cargo/**") },
    fileTree(repositoryRoot.resolve("shared/mimi-runtime")) { include("Cargo.toml", "Cargo.lock", "build.rs", "src/**", ".cargo/**") },
    fileTree(repositoryRoot.resolve("shared/mimi-android-jni")) { include("Cargo.toml", "Cargo.lock", "build.rs", "src/**", ".cargo/**") },
    fileTree(repositoryRoot.resolve(".cargo")) { include("config", "config.toml") },
    fileTree(repositoryRoot.resolve("shared/.cargo")) { include("config", "config.toml") },
    repositoryRoot.resolve("shared/provider-language-catalogs.json"),
    repositoryRoot.resolve("shared/translation-contracts.json"),
    repositoryRoot.resolve("scripts/build-shared-core.py"),
    repositoryRoot.resolve("scripts/verify-shared-core.py"),
    repositoryRoot.resolve("scripts/check-shared-runtime.py"),
    rootProject.file("shared-core.properties"),
)
val rustVersion = providers.exec { commandLine("rustc", "-vV") }.standardOutput.asText

val verifySharedRuntimeArchitecture = tasks.register<Exec>("verifySharedRuntimeArchitecture") {
    workingDir(repositoryRoot)
    commandLine(pythonExecutable.get(), "scripts/check-shared-runtime.py")
    // No output/cache marker: check current production sources on every build.
}
val verifySharedProviderCopy = tasks.register<Exec>("verifySharedProviderCopy") {
    workingDir(repositoryRoot)
    commandLine(pythonExecutable.get(), "-B", "scripts/sync-provider-copy.py", "--check")
    inputs.files(repositoryRoot.resolve("shared/provider-copy.json"),
        repositoryRoot.resolve("shared/translation-contracts.json"),
        repositoryRoot.resolve("scripts/sync-provider-copy.py"),
        repositoryRoot.resolve("src/lib/generated/providerCopy.ts"),
        repositoryRoot.resolve("src/lib/i18n.ts"),
        repositoryRoot.resolve("src/windows/settings/AlibabaRealtimeSettings.tsx"))
    inputs.files(fileTree(repositoryRoot.resolve("src/lib/locales")) { include("*.ts") },
        fileTree(repositoryRoot.resolve("src/lib/generated/providerCopy")) { include("*.ts") },
        fileTree("src/main/res") { include("values*/*.xml") })
    // Verify every build; generation is explicit and never silently fixes drift.
}
tasks.matching { it.name == "preBuild" }.configureEach {
    dependsOn(verifySharedRuntimeArchitecture)
    dependsOn(verifySharedProviderCopy)
}

val buildSharedCoreHost = tasks.register<Exec>("buildSharedCoreHost") {
    dependsOn(verifySharedRuntimeArchitecture)
    dependsOn(verifySharedProviderCopy)
    workingDir(repositoryRoot)
    commandLine(pythonExecutable.get(), "scripts/build-shared-core.py", "--platform", "host",
        "--profile", "debug", "--target-dir", sharedCoreTarget.absolutePath,
        "--output", sharedCoreHostOutput.get().asFile.absolutePath)
    inputs.files(sharedCoreNativeInputs).withPathSensitivity(PathSensitivity.RELATIVE)
    inputs.property("rustVersion", rustVersion)
    outputs.dir(sharedCoreHostOutput)
}

val sharedTestFixtures = tasks.register<Jar>("sharedTestFixtures") {
    // Keep shared Rust sources and build caches off the JVM test classpath.
    from(rootProject.file("../shared")) { include("*.json") }
    archiveFileName.set("shared-test-fixtures.jar")
    destinationDirectory.set(layout.buildDirectory.dir("generated/test-fixtures"))
}

android {
    namespace = "app.yuxino.mimi.android"
    compileSdk = 35
    buildToolsVersion = "36.0.0"
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

    buildFeatures {
        resValues = true
    }

    buildTypes {
        debug {
            applicationIdSuffix = ".dev"
            resValue("string", "app_name", "mimi Dev")
        }
        release {
            isDebuggable = false
            isMinifyEnabled = true
            isShrinkResources = true
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"))
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    sourceSets.getByName("main").assets.directories += rootProject.file("native-licenses").absolutePath

    packaging { jniLibs.useLegacyPackaging = false }
}

val pinnedNdk = androidComponents.sdkComponents.sdkDirectory.map { it.dir("ndk/$sharedCoreNdkVersion") }
for (variant in listOf("debug", "release")) {
    val suffix = variant.replaceFirstChar { it.uppercaseChar() }
    val output = layout.buildDirectory.dir("generated/shared-core/$variant/jniLibs")
    val nativeBuild = tasks.register<Exec>("buildSharedCore$suffix") {
        dependsOn(verifySharedRuntimeArchitecture)
        dependsOn(verifySharedProviderCopy)
        workingDir(repositoryRoot)
        commandLine(pythonExecutable.get(), "scripts/build-shared-core.py", "--platform", "android",
            "--profile", variant, "--target-dir", sharedCoreTarget.absolutePath,
            "--ndk", pinnedNdk.get().asFile.absolutePath, "--output", output.get().asFile.absolutePath)
        inputs.files(sharedCoreNativeInputs).withPathSensitivity(PathSensitivity.RELATIVE)
        inputs.property("rustVersion", rustVersion)
        inputs.property("ndkVersion", sharedCoreNdkVersion)
        outputs.dir(output)
    }
    android.sourceSets.getByName(variant).jniLibs.directories += output.get().asFile.absolutePath
    tasks.matching { it.name == "merge${suffix}JniLibFolders" }.configureEach { dependsOn(nativeBuild) }

    tasks.withType<KotlinCompile>().matching {
        it.name == "compile${suffix}UnitTestKotlin"
    }.configureEach {
        // Local JVM tests use org.json's implementation, including similar/keySet.
        // Kotlin 2.1.20+ otherwise puts Android's more limited JSON stubs first.
        val originalLibraries = project.files(libraries.from.toList())
        val jvmJson = configurations.named("${variant}UnitTestCompileClasspath").map { classpath ->
            classpath.incoming.artifactView {
                componentFilter { component ->
                    component is ModuleComponentIdentifier &&
                        component.group == "org.json" && component.module == "json"
                }
            }.files
        }
        libraries.setFrom(jvmJson, originalLibraries)
    }

    val apkName = if (variant == "release") "app-release-unsigned.apk" else "app-debug.apk"
    val apk = layout.buildDirectory.file("outputs/apk/$variant/$apkName")
    val verifyApk = tasks.register<Exec>("verifySharedCore${suffix}Apk") {
        dependsOn("package$suffix")
        workingDir(repositoryRoot)
        commandLine(pythonExecutable.get(), "scripts/verify-shared-core.py", "--apk", apk.get().asFile.absolutePath)
        inputs.file(apk)
        if (variant == "release") {
            val mapping = layout.buildDirectory.file("outputs/mapping/release/mapping.txt")
            args("--mapping", mapping.get().asFile.absolutePath)
            inputs.file(mapping)
        }
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
    // Tink 1.8 omits these annotation dependencies; R8 needs their definitions.
    compileOnly("com.google.code.findbugs:jsr305:3.0.2")
    testImplementation("junit:junit:4.13.2")
    testImplementation("org.json:json:20240303")
    testRuntimeOnly(files(sharedTestFixtures.flatMap { it.archiveFile }))
}
