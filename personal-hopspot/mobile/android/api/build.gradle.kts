plugins {
    alias(libs.plugins.android.library)
}

android {
    namespace = "org.personal.hopspot.api"
    compileSdk = 34

    defaultConfig {
        minSdk = 19
    }

    buildFeatures {
        aidl = true
        buildConfig = false
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_1_8
        targetCompatibility = JavaVersion.VERSION_1_8
    }
}
