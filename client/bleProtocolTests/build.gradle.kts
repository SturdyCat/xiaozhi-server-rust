import org.jetbrains.kotlin.gradle.dsl.JvmTarget

// Test-only ordinary KMP module: no Kuikly, KSP, @Page, renderer or native stubs.
plugins {
    kotlin("multiplatform")
}

kotlin {
    jvm {
        compilerOptions.jvmTarget.set(JvmTarget.JVM_17)
    }
    sourceSets {
        val commonMain by getting {
            // Compile exactly the production pure codec, not a copied test model.
            kotlin.srcDir(rootProject.file("shared/src/commonMain/kotlin"))
            kotlin.include("com/xiaozhi/admin/EchoMuseBleCodec.kt")
        }
        val commonTest by getting {
            dependencies {
                implementation(kotlin("test"))
            }
        }
    }
}
