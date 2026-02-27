#!/usr/bin/env bash
# Rust Release
#
# Builds the project for every target compatible with the current machine.
# Detects OS, architecture, and GPU capabilities (Vulkan, CUDA, Metal),
# then produces a release binary for each viable feature combination.
#
# Output: release/<name>-v<version>-<os>-<arch>[-<gpu>]
#
# Example outputs:
#   release/decree-v0.1.0-linux-amd64
#   release/decree-v0.1.0-linux-amd64-vulkan
#   release/decree-v0.1.0-linux-amd64-cuda
set -euo pipefail

# Parameters (decree injects these as env vars)
spec_file="${spec_file:-}"
message_file="${message_file:-}"
message_id="${message_id:-}"
message_dir="${message_dir:-}"
chain="${chain:-}"
seq="${seq:-}"

# ---------------------------------------------------------------------------
# Resolve project metadata
# ---------------------------------------------------------------------------
PKG_NAME=$(cargo metadata --no-deps --format-version=1 2>/dev/null \
  | python3 -c "import sys,json; print(json.load(sys.stdin)['packages'][0]['name'])" 2>/dev/null \
  || grep -m1 '^name' Cargo.toml | sed 's/.*"\(.*\)"/\1/')

PKG_VERSION=$(cargo metadata --no-deps --format-version=1 2>/dev/null \
  | python3 -c "import sys,json; print(json.load(sys.stdin)['packages'][0]['version'])" 2>/dev/null \
  || grep -m1 '^version' Cargo.toml | sed 's/.*"\(.*\)"/\1/')

# Map uname values to conventional release names
case "$(uname -s)" in
    Linux*)  OS="linux" ;;
    Darwin*) OS="darwin" ;;
    MINGW*|MSYS*|CYGWIN*) OS="windows" ;;
    *)       OS=$(uname -s | tr '[:upper:]' '[:lower:]') ;;
esac

case "$(uname -m)" in
    x86_64)  ARCH="amd64" ;;
    aarch64|arm64) ARCH="arm64" ;;
    armv7l)  ARCH="armv7" ;;
    i686)    ARCH="386" ;;
    *)       ARCH=$(uname -m) ;;
esac

PLATFORM="${OS}-${ARCH}"

echo "=== Rust Release Build ==="
echo "Package : ${PKG_NAME}"
echo "Version : ${PKG_VERSION}"
echo "Platform: ${PLATFORM}"
echo ""

# ---------------------------------------------------------------------------
# Detect available GPU backends
# ---------------------------------------------------------------------------
FEATURES=("default")

# Vulkan — check for vulkaninfo or libvulkan
if command -v vulkaninfo &>/dev/null && vulkaninfo --summary &>/dev/null; then
    FEATURES+=("vulkan")
    echo "[gpu] Vulkan detected"
elif [ -f /usr/lib64/libvulkan.so ] || [ -f /usr/lib/x86_64-linux-gnu/libvulkan.so.1 ]; then
    FEATURES+=("vulkan")
    echo "[gpu] Vulkan SDK detected (library present)"
fi

# CUDA — check for nvidia-smi or nvcc
if command -v nvidia-smi &>/dev/null && nvidia-smi &>/dev/null; then
    FEATURES+=("cuda")
    echo "[gpu] CUDA detected (nvidia-smi)"
elif command -v nvcc &>/dev/null; then
    FEATURES+=("cuda")
    echo "[gpu] CUDA detected (nvcc)"
fi

# Metal — macOS only
if [[ "$(uname -s)" == "Darwin" ]]; then
    FEATURES+=("metal")
    echo "[gpu] Metal detected (macOS)"
fi

# Ensure CUDA toolkit is discoverable by CMake and PATH
if [[ -d /usr/local/cuda ]]; then
    export PATH="/usr/local/cuda/bin:${PATH}"
    export CUDA_PATH="/usr/local/cuda"
    export CUDAToolkit_ROOT="/usr/local/cuda"

    # nvcc generates non-PIC host stubs by default, which rust-lld rejects
    # in PIE builds. Wrap nvcc to inject -fPIC via -Xcompiler since env vars
    # (CUDAFLAGS, NVCC_PREPEND_FLAGS) don't propagate through cmake.
    NVCC_WRAPPER="$(mktemp)"
    cat > "${NVCC_WRAPPER}" << 'NVCC_EOF'
#!/bin/bash
exec /usr/local/cuda/bin/nvcc -Xcompiler -fPIC "$@"
NVCC_EOF
    chmod +x "${NVCC_WRAPPER}"
    export CMAKE_CUDA_COMPILER="${NVCC_WRAPPER}"
fi

echo ""
echo "Build variants: ${FEATURES[*]}"
echo ""

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------
sha256() {
    if command -v sha256sum &>/dev/null; then
        sha256sum "$1" | cut -d' ' -f1
    else
        shasum -a 256 "$1" | cut -d' ' -f1
    fi
}

emit_artifact() {
    local dest="$1" artifact="$2"
    chmod +x "${dest}"
    local size sha
    size=$(du -h "${dest}" | cut -f1)
    sha=$(sha256 "${dest}")
    echo "  -> ${dest}  (${size}, sha256:${sha})"
    MANIFEST+="${sha}  ${artifact}\n"
}

# ---------------------------------------------------------------------------
# macOS universal binary setup
# ---------------------------------------------------------------------------
MACOS_UNIVERSAL=false
if [[ "${OS}" == "darwin" ]]; then
    MACOS_TARGETS=("aarch64-apple-darwin" "x86_64-apple-darwin")
    MACOS_UNIVERSAL=true
    echo "[universal] Will build for: ${MACOS_TARGETS[*]}"
    for t in "${MACOS_TARGETS[@]}"; do
        rustup target add "$t" 2>/dev/null || true
    done
    echo ""
fi

# ---------------------------------------------------------------------------
# Build each variant
# ---------------------------------------------------------------------------
RELEASE_DIR="release"
mkdir -p "${RELEASE_DIR}"

MANIFEST=""
FAILED=()

for feat in "${FEATURES[@]}"; do
    if [ "$feat" = "default" ]; then
        SUFFIX=""
        FEATURE_FLAG=""
        LABEL="default (cpu-only)"
    else
        SUFFIX="-${feat}"
        FEATURE_FLAG="--features ${feat}"
        LABEL="${feat}"
    fi

    # Clean llama-cpp-sys-2 build cache between variants so CMake
    # reconfigures with the correct GPU backend flags.
    rm -rf target/release/build/llama-cpp-sys-2-*/

    if [[ "${MACOS_UNIVERSAL}" == "true" ]]; then
        # --- macOS: build both archs, then lipo into a universal binary ---
        ARCH_BINS=()
        ARCH_OK=true

        for target in "${MACOS_TARGETS[@]}"; do
            case "$target" in
                aarch64*) tarch="arm64" ;;
                x86_64*)  tarch="amd64" ;;
                *)        tarch="$target" ;;
            esac

            ARTIFACT="${PKG_NAME}-v${PKG_VERSION}-darwin-${tarch}${SUFFIX}"
            DEST="${RELEASE_DIR}/${ARTIFACT}"

            echo "--- Building: ${ARTIFACT} [${LABEL}] ---"

            # Also clean target-specific llama cache
            rm -rf "target/${target}/release/build/llama-cpp-sys-2-*/"

            # shellcheck disable=SC2086
            if cargo build --release --target "$target" ${FEATURE_FLAG} 2>&1 \
                    | tee "${message_dir:-.}/build-${feat}-${tarch}.log"; then
                cp "target/${target}/release/${PKG_NAME}" "${DEST}"
                emit_artifact "${DEST}" "${ARTIFACT}"
                ARCH_BINS+=("${DEST}")
            else
                echo "  !! Build FAILED for ${target} variant: ${LABEL}"
                FAILED+=("${ARTIFACT}")
                ARCH_OK=false
            fi
            echo ""
        done

        # Combine into universal binary if both archs succeeded
        if [[ "${ARCH_OK}" == "true" ]] && [ ${#ARCH_BINS[@]} -eq 2 ]; then
            UNIVERSAL_ARTIFACT="${PKG_NAME}-v${PKG_VERSION}-darwin-universal${SUFFIX}"
            UNIVERSAL_DEST="${RELEASE_DIR}/${UNIVERSAL_ARTIFACT}"
            echo "--- Combining: ${UNIVERSAL_ARTIFACT} [universal ${LABEL}] ---"
            lipo -create "${ARCH_BINS[@]}" -output "${UNIVERSAL_DEST}"
            emit_artifact "${UNIVERSAL_DEST}" "${UNIVERSAL_ARTIFACT}"
            echo ""
        fi
    else
        # --- Linux / Windows: single-arch build ---
        ARTIFACT="${PKG_NAME}-v${PKG_VERSION}-${PLATFORM}${SUFFIX}"
        DEST="${RELEASE_DIR}/${ARTIFACT}"

        echo "--- Building: ${ARTIFACT} [${LABEL}] ---"

        # shellcheck disable=SC2086
        if cargo build --release ${FEATURE_FLAG} 2>&1 | tee "${message_dir:-.}/build-${feat}.log"; then
            cp "target/release/${PKG_NAME}" "${DEST}"
            emit_artifact "${DEST}" "${ARTIFACT}"
        else
            echo "  !! Build FAILED for variant: ${LABEL}"
            FAILED+=("${ARTIFACT}")
        fi

        echo ""
    fi
done

# ---------------------------------------------------------------------------
# Write checksums manifest
# ---------------------------------------------------------------------------
CHECKSUMS_FILE="${RELEASE_DIR}/${PKG_NAME}-v${PKG_VERSION}-${PLATFORM}.sha256"
echo -e "${MANIFEST}" > "${CHECKSUMS_FILE}"
echo "=== Checksums written to ${CHECKSUMS_FILE} ==="
cat "${CHECKSUMS_FILE}"

# ---------------------------------------------------------------------------
# Summary
# ---------------------------------------------------------------------------
echo ""
echo "=== Release Summary ==="
echo "Output directory: ${RELEASE_DIR}/"
ls -lh "${RELEASE_DIR}/"

if [ ${#FAILED[@]} -gt 0 ]; then
    echo ""
    echo "Skipped variants (build not available on this machine):"
    for f in "${FAILED[@]}"; do
        echo "  - ${f}"
    done
fi

# Fail only if the default (cpu-only) build failed
DEFAULT_ARTIFACT="${RELEASE_DIR}/${PKG_NAME}-v${PKG_VERSION}-${PLATFORM}"
if [[ "${MACOS_UNIVERSAL}" == "true" ]]; then
    DEFAULT_ARTIFACT="${RELEASE_DIR}/${PKG_NAME}-v${PKG_VERSION}-darwin-universal"
fi
if [ ! -f "${DEFAULT_ARTIFACT}" ]; then
    echo "FATAL: default build failed"
    exit 1
fi

echo ""
echo "Release complete."
