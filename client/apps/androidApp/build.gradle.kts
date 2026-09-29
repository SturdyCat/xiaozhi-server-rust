plugins {
    id("com.android.application")
    kotlin("android")
}

val kuiklyVersion: String = property("kuiklyVersion") as String

android {
    namespace = "com.xiaozhi.admin"
    compileSdk = 35
    defaultConfig {
        applicationId = "com.xiaozhi.admin"
        minSdk = 21
        targetSdk = 35
        versionCode = 1
        versionName = "1.0"
        // 仅 arm64（不含 x86/x64）：原生 so 与 KMP 框架均只编 arm64-v8a
        ndk {
            abiFilters += listOf("arm64-v8a")
        }
    }
    buildTypes {
        release {
            isMinifyEnabled = false
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
        }
    }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
}

dependencies {
    implementation(project(":shared"))
    // Kuikly 渲染器 + 核心（版本必须与 shared 的 core 一致）
    implementation("com.tencent.kuikly-open:core-render-android:$kuiklyVersion")
    implementation("com.tencent.kuikly-open:core:$kuiklyVersion")

    implementation("androidx.core:core-ktx:1.16.0")
    implementation("androidx.appcompat:appcompat:1.7.1")
    implementation("com.google.android.material:material:1.13.0")
    implementation("androidx.constraintlayout:constraintlayout:2.2.1")
}
