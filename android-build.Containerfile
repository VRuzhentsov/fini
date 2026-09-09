# Local Android build environment: JDK 17 (matching ci.yml's setup-java
# version) plus Node, for hosts whose system JDK is too new for the Gradle
# version this project pins -- Gradle's bundled Kotlin DSL compiler fails to
# parse some newer JDK version strings before it ever reads our build files.
#
# The concrete failure this exists to avoid, on a host whose `java` resolves to
# the flatpak Android Studio's bundled JBR (which auto-updates itself):
#
#     A problem occurred configuring project ':buildSrc'.
#     > 25.0.2
#
# Everything else (Android SDK/NDK, cargo, rustup, the repo itself) is
# bind-mounted from the host at run time, so this image stays tiny and holds
# no project state. `adb` deliberately stays on the host: the USB device and
# the adb server live there, and reaching them from inside would mean handing
# the container far more device access than a build needs.
FROM docker.io/library/eclipse-temurin:17-jdk

# gcc/build-essential is required even though nothing here targets the host:
# cargo compiles each crate's *build script* for the host triple first, and
# those need a working host linker (`cc`). Without it the Android build fails
# at "linker `cc` not found" while compiling build scripts, long before it
# reaches any Android cross-compilation.
RUN apt-get update \
    && apt-get install -y --no-install-recommends curl ca-certificates python3 build-essential pkg-config \
    && curl -fsSL https://deb.nodesource.com/setup_24.x | bash - \
    && apt-get install -y --no-install-recommends nodejs \
    && rm -rf /var/lib/apt/lists/*
