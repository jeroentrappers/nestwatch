import org.gradle.api.tasks.Exec

plugins {
    alias(libs.plugins.android.application)
    alias(libs.plugins.kotlin.compose)
}

/** Cargo workspace root (two levels above this project). */
val workspaceDir: File = rootDir.resolve("../..").canonicalFile
val abis: List<String> = (findProperty("lull.abis") as String? ?: "arm64-v8a,armeabi-v7a,x86_64")
    .split(',').map { it.trim() }.filter { it.isNotEmpty() }
val rustJniDir = layout.buildDirectory.dir("rustJniLibs")
val bindingsDir = layout.buildDirectory.dir("generated/uniffi/kotlin")
/** `cargo` from PATH (with ~/.cargo/bin for cargo-ndk), or `$CARGO`. */
val cargo: String = System.getenv("CARGO") ?: "cargo"

android {
    namespace = "org.lull.android"
    compileSdk = 36
    ndkVersion = "30.0.16248370"

    defaultConfig {
        applicationId = "org.lull.android"
        minSdk = 23
        targetSdk = 36
        versionCode = 1
        versionName = "0.1.0"
        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
        ndk { abiFilters += abis }
    }

    buildTypes {
        release {
            isMinifyEnabled = true
            isShrinkResources = true
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
        }
    }

    compileOptions {
        // JNA uses java.util.function and streams, which API 23 lacks.
        isCoreLibraryDesugaringEnabled = true
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    buildFeatures {
        compose = true
        buildConfig = true
    }

    sourceSets {
        getByName("main") {
            jniLibs.directories.add(rustJniDir.get().asFile.path)
            kotlin.directories.add(bindingsDir.get().asFile.path)
        }
    }

    packaging {
        jniLibs.useLegacyPackaging = false
    }

    testOptions {
        unitTests.isReturnDefaultValues = true
    }
}

/**
 * Builds lull-ffi for every ABI with cargo-ndk (release profile: Argon2id
 * and the media chain are far too slow unoptimised) into rustJniLibs.
 */
val cargoNdkBuild = tasks.register<Exec>("cargoNdkBuild") {
    group = "rust"
    description = "Builds liblull_ffi.so for ${abis.joinToString()} with cargo-ndk"
    workingDir = workspaceDir
    inputs.files(fileTree(workspaceDir.resolve("crates")) { exclude("**/target/**") })
    inputs.file(workspaceDir.resolve("Cargo.lock"))
    inputs.property("abis", abis)
    outputs.dir(rustJniDir)
    val args = mutableListOf(cargo, "ndk", "--platform", "23")
    abis.forEach { args += listOf("-t", it) }
    args += listOf("-o", rustJniDir.get().asFile.path, "build", "--release", "-p", "lull-ffi", "--lib")
    commandLine(args)
    environment("PATH", "${System.getProperty("user.home")}/.cargo/bin:${System.getenv("PATH")}")
    androidComponents.sdkComponents.ndkDirectory.orNull?.let { environment("ANDROID_NDK_HOME", it.asFile.path) }
}

/** Generates the Kotlin bindings from the built library (UniFFI library mode). */
val uniffiBindings = tasks.register<Exec>("uniffiBindings") {
    group = "rust"
    description = "Generates the UniFFI Kotlin bindings"
    dependsOn(cargoNdkBuild)
    workingDir = workspaceDir
    val lib = rustJniDir.get().asFile.resolve("${abis.first()}/liblull_ffi.so")
    inputs.dir(rustJniDir)
    outputs.dir(bindingsDir)
    commandLine(
        cargo, "run", "--quiet", "-p", "lull-ffi", "--features", "bindgen", "--bin", "uniffi-bindgen", "--",
        "generate", "--library", lib.path, "--language", "kotlin", "--no-format",
        "--out-dir", bindingsDir.get().asFile.path,
    )
    environment("PATH", "${System.getProperty("user.home")}/.cargo/bin:${System.getenv("PATH")}")
}

tasks.named("preBuild") { dependsOn(uniffiBindings) }

dependencies {
    implementation(libs.androidx.core.ktx)
    implementation(libs.androidx.annotation)
    implementation(libs.androidx.activity.compose)
    implementation(libs.androidx.lifecycle.runtime.compose)
    implementation(libs.androidx.lifecycle.service)
    implementation(libs.androidx.lifecycle.process)
    implementation(platform(libs.compose.bom))
    implementation(libs.compose.ui)
    implementation(libs.compose.ui.graphics)
    implementation(libs.compose.ui.tooling.preview)
    implementation(libs.compose.foundation)
    implementation(libs.compose.material3)
    implementation(libs.kotlinx.coroutines.android)
    implementation(libs.camera.core)
    implementation(libs.camera.camera2)
    implementation(libs.camera.lifecycle)
    implementation(libs.camera.view)
    implementation(libs.zxing.core)
    implementation("${libs.jna.get()}@aar")
    debugImplementation(libs.compose.ui.tooling)
    coreLibraryDesugaring(libs.desugar.jdk.libs)
    testImplementation(libs.junit)
}
