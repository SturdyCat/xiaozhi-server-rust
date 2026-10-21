import org.jetbrains.kotlin.gradle.dsl.JvmTarget

// Compile the production pure models, never a copied implementation or native UI stub.
plugins {
    kotlin("multiplatform")
}

kotlin {
    jvm {
        compilerOptions.jvmTarget.set(JvmTarget.JVM_17)
    }
    sourceSets {
        val commonMain by getting {
            kotlin.srcDir(rootProject.file("shared/src/commonMain/kotlin"))
            kotlin.include("com/xiaozhi/admin/UiContracts.kt")
            kotlin.include("com/xiaozhi/admin/PipelinePanels.kt")
            kotlin.include("com/xiaozhi/admin/PipelineFlow.kt")
            kotlin.include("com/xiaozhi/admin/PipelineWorkflow.kt")
            kotlin.include("com/xiaozhi/admin/NodeTestModels.kt")
            kotlin.include("com/xiaozhi/admin/ConfigSections.kt")
        }
        val commonTest by getting {
            dependencies { implementation(kotlin("test")) }
        }
    }
}
