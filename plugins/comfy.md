---
name: comfy
machine:
  description: Render the message body on ComfyUI with the workflow `method` names, wait for the render, and save it to the repo path in `output`.
  data:
    method:
      type: string
      default: ''
    output:
      type: string
      default: ''
    input_image:
      type: string
      default: ''
    width:
      type: int
      default: 0
    height:
      type: int
      default: 0
    seed:
      type: int
      default: -1
  onentry:
  - precheck
  initial: build
  states:
    build:
      invoke: build
      transitions:
        done: submit
    submit:
      invoke:
        script:
          name: submit
          attempts: 3
          timeout: 2m
      transitions:
        done: await
    await:
      invoke:
        script:
          name: await
          timeout: 3h
      transitions:
        done: fetch
    fetch:
      invoke:
        script:
          name: fetch
          attempts: 3
          timeout: 10m
      transitions:
        done: done
    done:
      final: true
    failed:
      final: true
scripts:
  await.sh: |
    #!/usr/bin/env bash
    # comfy's await: poll GET /history/<prompt id> every COMFY_POLL_S seconds until
    # ComfyUI lists the prompt there, which it does once the prompt finishes; keep
    # the entry in runs/<id>/comfy-history.json for fetch, and fail if the prompt
    # did not succeed. The state's timeout bounds the wait.
    set -euo pipefail
    COMFY_URL="${COMFY_URL:-http://127.0.0.1:8188}"
    COMFY_POLL_S="${COMFY_POLL_S:-5}"
    id=$(cat "${DECREE_RUN_DIR}/comfy-prompt-id")

    echo "waiting for prompt ${id}"
    until history=$(curl -fsS --max-time 30 "${COMFY_URL}/history/${id}") \
      && jq -e --arg id "${id}" 'has($id)' <<<"${history}" >/dev/null; do
      sleep "${COMFY_POLL_S}"
    done
    jq --arg id "${id}" '.[$id]' <<<"${history}" > "${DECREE_RUN_DIR}/comfy-history.json"

    if ! jq -e '.status.status_str == "success"' "${DECREE_RUN_DIR}/comfy-history.json" >/dev/null; then
      echo "await: prompt ${id} failed: $(jq -c .status "${DECREE_RUN_DIR}/comfy-history.json")" >&2
      exit 1
    fi
    echo "prompt ${id} finished"
  build.sh: |
    #!/usr/bin/env bash
    # comfy's build: patch .decree/machines/comfy/scripts/<method>.json with the message and its
    # params, and save it as runs/<id>/comfy-payload.json for submit. The method is
    # $METHOD when a wrapper script sets it, else the `method` param. Every workflow is
    # patched the same way, by node type, so a new method is a new workflow file:
    #   the prompt (the message body)  every CLIPTextEncode no `negative` input uses
    #   width, height (0 keeps)        every node with numeric width and height
    #   seed (-1 keeps)                every node with a numeric seed or noise_seed
    #   the file name prefix           every node with a filename_prefix
    # input_image is required when the workflow has a LoadImage node; submit
    # uploads it and points LoadImage at it.
    set -euo pipefail
    workflows=.decree/machines/comfy/scripts
    method="${METHOD:-${DECREE_DATA_METHOD}}"

    methods() {
      local f
      for f in "${workflows}"/*.json; do basename "${f}" .json; done | paste -sd ',' | sed 's/,/, /g'
    }
    fail() {
      echo "build: $1" >&2
      exit 1
    }

    if [ -z "${method}" ]; then
      fail "method is required; the methods are: $(methods)"
    fi
    workflow="${workflows}/${method}.json"
    if [[ "${method}" == */* ]] || [ ! -f "${workflow}" ]; then
      fail "unknown method '${method}'; the methods are: $(methods)"
    fi
    [ -n "${DECREE_DATA_OUTPUT}" ] || fail "output is required: a repo path without extension"

    # The message body is the prompt: everything after the frontmatter, leading
    # blank lines removed.
    prompt=$(awk 'NR == 1 && /^---$/ { fm = 1; next } fm == 1 && /^---$/ { fm = 2; next } fm != 1' "${DECREE_MESSAGE}" \
      | sed '/\S/,$!d')
    [ -n "${prompt}" ] || fail "the message body, the prompt, is empty"

    if jq -e '[.prompt[] | select(.class_type == "LoadImage")] | length > 0' "${workflow}" >/dev/null; then
      [ -n "${DECREE_DATA_INPUT_IMAGE}" ] || fail "input_image is required by method ${method}"
      [ -f "${DECREE_PROJECT_ROOT}/${DECREE_DATA_INPUT_IMAGE}" ] \
        || fail "input_image ${DECREE_DATA_INPUT_IMAGE} is not a file in the repo"
    fi

    # Diffusion models need sizes in multiples of 16: round to the nearest.
    align16() { echo $(( (($1 + 8) / 16) * 16 )); }
    width=0
    height=0
    [ "${DECREE_DATA_WIDTH}" -le 0 ] || width=$(align16 "${DECREE_DATA_WIDTH}")
    [ "${DECREE_DATA_HEIGHT}" -le 0 ] || height=$(align16 "${DECREE_DATA_HEIGHT}")

    echo "=== ${method} ==="
    echo "  prompt:  ${prompt:0:80}"
    echo "  output:  ${DECREE_DATA_OUTPUT}"
    [ -z "${DECREE_DATA_INPUT_IMAGE}" ] || echo "  image:   ${DECREE_DATA_INPUT_IMAGE}"
    [ "${width}${height}" = 00 ] || echo "  size:    ${width}x${height} (0 keeps the workflow's)"
    [ "${DECREE_DATA_SEED}" -lt 0 ] || echo "  seed:    ${DECREE_DATA_SEED}"

    jq \
      --arg text "${prompt}" \
      --argjson width "${width}" \
      --argjson height "${height}" \
      --argjson seed "${DECREE_DATA_SEED}" \
      --arg prefix "decree_${DECREE_MESSAGE_ID}" \
      '
      def number($key): (.inputs[$key] | type) == "number";
      [.prompt[] | .inputs.negative? | arrays | .[0]] as $negative
      | {prompt: (.prompt | with_entries(
          (if .value.class_type == "CLIPTextEncode" and (.key | IN($negative[]) | not)
           then .value.inputs.text = $text else . end)
          | (if $width > 0 and (.value | number("width")) then .value.inputs.width = $width else . end)
          | (if $height > 0 and (.value | number("height")) then .value.inputs.height = $height else . end)
          | (if $seed >= 0 and (.value | number("seed")) then .value.inputs.seed = $seed else . end)
          | (if $seed >= 0 and (.value | number("noise_seed")) then .value.inputs.noise_seed = $seed else . end)
          | (if (.value.inputs.filename_prefix | type) == "string" then .value.inputs.filename_prefix = $prefix else . end)
        ))}
      ' "${workflow}" > "${DECREE_RUN_DIR}/comfy-payload.json"
    echo "wrote comfy-payload.json"
  fetch.sh: |
    #!/usr/bin/env bash
    # comfy's fetch: download each file the prompt saved (GET /view) to `output`
    # plus the file's extension, or `output`-<n> plus its extension when there are
    # several. Safe to repeat: each download goes to a .part file, then moves.
    set -euo pipefail
    COMFY_URL="${COMFY_URL:-http://127.0.0.1:8188}"
    history="${DECREE_RUN_DIR}/comfy-history.json"
    output="${DECREE_PROJECT_ROOT}/${DECREE_DATA_OUTPUT}"

    # Each node's outputs map a kind (images, gifs, ...) to a list of
    # {filename, subfolder, type}; "output" files are saved, "temp" are previews.
    mapfile -t files < <(jq -c '.outputs[][] | arrays | .[] | objects
      | select(.filename and .type == "output")' "${history}")
    if [ "${#files[@]}" -eq 0 ]; then
      echo "fetch: the prompt saved no file" >&2
      exit 1
    fi

    mkdir -p "$(dirname "${output}")"
    n=0
    for file in "${files[@]}"; do
      n=$((n + 1))
      filename=$(jq -r .filename <<<"${file}")
      if [ "${#files[@]}" -eq 1 ]; then
        dest="${output}.${filename##*.}"
      else
        dest="${output}-${n}.${filename##*.}"
      fi
      curl -fsS --max-time 600 -G "${COMFY_URL}/view" \
        --data-urlencode "filename=${filename}" \
        --data-urlencode "subfolder=$(jq -r '.subfolder // ""' <<<"${file}")" \
        --data-urlencode "type=output" \
        -o "${dest}.part"
      mv "${dest}.part" "${dest}"
      echo "saved ${dest#"${DECREE_PROJECT_ROOT}/"}"
    done
  image_flux2_text_image.json: |
    {
      "client_id": "70b193468f844b99ada3c47dd94e287e",
      "prompt": {
        "6": {
          "inputs": {
            "text": "Photorealistic satellite view from 400 kilometers (250 miles) above Earth's surface, capturing the eastern Mediterranean region centered on Jerusalem. The perspective matches actual ISS astronaut photography: the curvature of Earth is subtly visible at this altitude, with the black void of space above transitioning to the thin blue line of Earth's atmosphere at the horizon. Jerusalem appears as a distinct point on the Judaean Mountains plateau at 31.77°N, 35.22°E, elevation 760 meters (2,490 feet). The city is visible as a small but clearly defined urban area—approximately 12-15 acres of dense settlement—showing as a tan/gray cluster against the brown and tan limestone terrain of the surrounding hills. The topography is scientifically accurate: the Judaean Mountains form a central ridge with Jerusalem on the plateau, the terrain slopes dramatically eastward into the Jordan Rift Valley (visible as a dark, shadowed depression), and descends more gradually westward toward the Mediterranean coastal plain. The Mediterranean Sea appears as a vast dark blue body of water on the western horizon, approximately 60 km from Jerusalem, its coastline visible as a subtle line where land meets sea. The Dead Sea is clearly visible 35 km to the east as a distinctive turquoise-blue body of water in the deepest terrestrial depression on Earth (430 meters below sea level), creating a stark color contrast with the surrounding desert terrain. The Jordan River valley appears as a dark, vegetation-rich corridor trending north-south between Jerusalem and the Dead Sea. The terrain shows realistic geological features: exposed limestone bedrock, terraced hillsides, sparse Mediterranean vegetation (appearing as muted green patches), and the characteristic brown and tan earth tones of the Levantine landscape. Small towns and villages appear as tiny points of human settlement scattered across the region. Highway corridors show as thin lines connecting population centers. The image captures the dramatic elevation change from the coastal plains (sea level) to the mountain plateau (760m) to the Dead Sea depression (-430m). Atmospheric perspective is scientifically accurate: distant terrain shows natural haze and reduced contrast, while the foreground maintains sharp detail. The sky grades from deep black at the top of the frame through navy blue to the thin atmospheric halo at Earth's limb. Cloud formations appear realistic for the region—sparse, wispy cirrus clouds typical of the eastern Mediterranean climate. The sun angle creates scientifically accurate shadows based on time of day, emphasizing the three-dimensional topography. The overall image conveys the remarkable geographic reality: Jerusalem as a small but significant human settlement perched on a mountain plateau between the Mediterranean Sea and the Dead Sea, positioned at the crossroads of ancient civilizations yet geographically isolated by dramatic terrain.",
            "clip": ["38", 0]
          },
          "class_type": "CLIPTextEncode",
          "_meta": { "title": "CLIP Text Encode (Positive Prompt)" }
        },
        "8": {
          "inputs": { "samples": ["13", 0], "vae": ["10", 0] },
          "class_type": "VAEDecode",
          "_meta": { "title": "VAE Decode" }
        },
        "9": {
          "inputs": { "filename_prefix": "2026.02.22_flux2", "images": ["8", 0] },
          "class_type": "SaveImage",
          "_meta": { "title": "Save Image" }
        },
        "10": {
          "inputs": { "vae_name": "flux2-vae.safetensors" },
          "class_type": "VAELoader",
          "_meta": { "title": "Load VAE" }
        },
        "12": {
          "inputs": {
            "unet_name": "flux2_dev_fp8mixed.safetensors",
            "weight_dtype": "default"
          },
          "class_type": "UNETLoader",
          "_meta": { "title": "Load Diffusion Model" }
        },
        "13": {
          "inputs": {
            "noise": ["25", 0],
            "guider": ["22", 0],
            "sampler": ["16", 0],
            "sigmas": ["48", 0],
            "latent_image": ["47", 0]
          },
          "class_type": "SamplerCustomAdvanced",
          "_meta": { "title": "SamplerCustomAdvanced" }
        },
        "16": {
          "inputs": { "sampler_name": "euler" },
          "class_type": "KSamplerSelect",
          "_meta": { "title": "KSamplerSelect" }
        },
        "22": {
          "inputs": { "model": ["12", 0], "conditioning": ["39", 0] },
          "class_type": "BasicGuider",
          "_meta": { "title": "BasicGuider" }
        },
        "25": {
          "inputs": { "noise_seed": 730539084012896 },
          "class_type": "RandomNoise",
          "_meta": { "title": "RandomNoise" }
        },
        "26": {
          "inputs": { "guidance": 4, "conditioning": ["6", 0] },
          "class_type": "FluxGuidance",
          "_meta": { "title": "FluxGuidance" }
        },
        "38": {
          "inputs": {
            "clip_name": "mistral_3_small_flux2_bf16.safetensors",
            "type": "flux2",
            "device": "default"
          },
          "class_type": "CLIPLoader",
          "_meta": { "title": "Load CLIP" }
        },
        "39": {
          "inputs": { "conditioning": ["43", 0] },
          "class_type": "ReferenceLatent",
          "_meta": { "title": "ReferenceLatent" }
        },
        "43": {
          "inputs": { "conditioning": ["26", 0], "latent": ["44", 0] },
          "class_type": "ReferenceLatent",
          "_meta": { "title": "ReferenceLatent" }
        },
        "44": {
          "inputs": { "pixels": ["45", 0], "vae": ["10", 0] },
          "class_type": "VAEEncode",
          "_meta": { "title": "VAE Encode" }
        },
        "45": {
          "inputs": {
            "upscale_method": "lanczos",
            "megapixels": 1,
            "resolution_steps": 1,
            "image": ["46", 0]
          },
          "class_type": "ImageScaleToTotalPixels",
          "_meta": { "title": "ImageScaleToTotalPixels" }
        },
        "46": {
          "inputs": { "image": "example.png" },
          "class_type": "LoadImage",
          "_meta": { "title": "Load Image" }
        },
        "47": {
          "inputs": { "width": 960, "height": 1664, "batch_size": 1 },
          "class_type": "EmptyFlux2LatentImage",
          "_meta": { "title": "Empty Flux 2 Latent" }
        },
        "48": {
          "inputs": { "steps": 20, "width": 960, "height": 1664 },
          "class_type": "Flux2Scheduler",
          "_meta": { "title": "Flux2Scheduler" }
        }
      },
      "extra_data": {
        "extra_pnginfo": {
          "workflow": {
            "id": "7c048efb-a059-44e2-970a-43e1eb472d0d",
            "revision": 0,
            "last_node_id": 67,
            "last_link_id": 165,
            "nodes": [
              {
                "id": 48,
                "type": "Flux2Scheduler",
                "pos": [-190.01410752108893, 313.9860761244213],
                "size": [266.8166809082031, 144],
                "flags": {},
                "order": 13,
                "mode": 0,
                "inputs": [
                  {
                    "name": "width",
                    "type": "INT",
                    "widget": { "name": "width" },
                    "link": 136
                  },
                  {
                    "name": "height",
                    "type": "INT",
                    "widget": { "name": "height" },
                    "link": 138
                  }
                ],
                "outputs": [{ "name": "SIGMAS", "type": "SIGMAS", "links": [132] }],
                "properties": {
                  "cnr_id": "comfy-core",
                  "ver": "0.3.71",
                  "Node name for S&R": "Flux2Scheduler"
                },
                "widgets_values": [20, 960, 1664]
              },
              {
                "id": 22,
                "type": "BasicGuider",
                "pos": [-190.01410752108893, 85.99478455239108],
                "size": [266.8166809082031, 72],
                "flags": {},
                "order": 22,
                "mode": 0,
                "inputs": [
                  { "name": "model", "type": "MODEL", "link": 165 },
                  { "name": "conditioning", "type": "CONDITIONING", "link": 153 }
                ],
                "outputs": [
                  {
                    "name": "GUIDER",
                    "type": "GUIDER",
                    "slot_index": 0,
                    "links": [30]
                  }
                ],
                "properties": {
                  "cnr_id": "comfy-core",
                  "ver": "0.3.71",
                  "Node name for S&R": "BasicGuider"
                },
                "widgets_values": []
              },
              {
                "id": 40,
                "type": "VAEEncode",
                "pos": [-485.11167714363637, 1058.0069731984254],
                "size": [228.75, 8],
                "flags": { "collapsed": true },
                "order": 17,
                "mode": 4,
                "inputs": [
                  { "name": "pixels", "type": "IMAGE", "link": 122 },
                  { "name": "vae", "type": "VAE", "link": 120 }
                ],
                "outputs": [{ "name": "LATENT", "type": "LATENT", "links": [121] }],
                "properties": {
                  "cnr_id": "comfy-core",
                  "ver": "0.3.71",
                  "Node name for S&R": "VAEEncode"
                },
                "widgets_values": []
              },
              {
                "id": 42,
                "type": "LoadImage",
                "pos": [-645.9966750916013, 481.9814265758871],
                "size": [328.8999938964844, 344],
                "flags": {},
                "order": 0,
                "mode": 4,
                "inputs": [],
                "outputs": [
                  { "name": "IMAGE", "type": "IMAGE", "links": [123] },
                  { "name": "MASK", "type": "MASK", "links": null }
                ],
                "properties": {
                  "cnr_id": "comfy-core",
                  "ver": "0.3.71",
                  "Node name for S&R": "LoadImage"
                },
                "widgets_values": ["image_flux2_input_image.png", "image"]
              },
              {
                "id": 44,
                "type": "VAEEncode",
                "pos": [-1005.9891879674415, 1058.0069731984254],
                "size": [225, 8],
                "flags": { "collapsed": true },
                "order": 18,
                "mode": 0,
                "inputs": [
                  { "name": "pixels", "type": "IMAGE", "link": 126 },
                  { "name": "vae", "type": "VAE", "link": 127 }
                ],
                "outputs": [{ "name": "LATENT", "type": "LATENT", "links": [125] }],
                "properties": {
                  "cnr_id": "comfy-core",
                  "ver": "0.3.71",
                  "Node name for S&R": "VAEEncode"
                },
                "widgets_values": []
              },
              {
                "id": 39,
                "type": "ReferenceLatent",
                "pos": [-554.3953085097025, 1106.0103791145702],
                "size": [237.25, 72],
                "flags": {},
                "order": 21,
                "mode": 0,
                "inputs": [
                  { "name": "conditioning", "type": "CONDITIONING", "link": 145 },
                  { "name": "latent", "shape": 7, "type": "LATENT", "link": 121 }
                ],
                "outputs": [
                  { "name": "CONDITIONING", "type": "CONDITIONING", "links": [153] }
                ],
                "properties": {
                  "cnr_id": "comfy-core",
                  "ver": "0.3.71",
                  "Node name for S&R": "ReferenceLatent"
                },
                "widgets_values": []
              },
              {
                "id": 26,
                "type": "FluxGuidance",
                "pos": [-621.9949098393369, 265.9825456198922],
                "size": [380.8833312988281, 80],
                "flags": {},
                "order": 19,
                "mode": 0,
                "inputs": [
                  { "name": "conditioning", "type": "CONDITIONING", "link": 41 }
                ],
                "outputs": [
                  {
                    "name": "CONDITIONING",
                    "type": "CONDITIONING",
                    "slot_index": 0,
                    "links": [144]
                  }
                ],
                "properties": {
                  "cnr_id": "comfy-core",
                  "ver": "0.3.71",
                  "Node name for S&R": "FluxGuidance"
                },
                "widgets_values": [4],
                "color": "#233",
                "bgcolor": "#355"
              },
              {
                "id": 16,
                "type": "KSamplerSelect",
                "pos": [-190.01410752108893, 193.99425617748443],
                "size": [266.8166809082031, 82.66666412353516],
                "flags": {},
                "order": 1,
                "mode": 0,
                "inputs": [],
                "outputs": [
                  { "name": "SAMPLER", "type": "SAMPLER", "links": [19] }
                ],
                "properties": {
                  "cnr_id": "comfy-core",
                  "ver": "0.3.71",
                  "Node name for S&R": "KSamplerSelect"
                },
                "widgets_values": ["euler"]
              },
              {
                "id": 43,
                "type": "ReferenceLatent",
                "pos": [-1089.9868943402703, 1106.0103791145702],
                "size": [237.25, 72],
                "flags": {},
                "order": 20,
                "mode": 0,
                "inputs": [
                  { "name": "conditioning", "type": "CONDITIONING", "link": 144 },
                  { "name": "latent", "shape": 7, "type": "LATENT", "link": 125 }
                ],
                "outputs": [
                  { "name": "CONDITIONING", "type": "CONDITIONING", "links": [145] }
                ],
                "properties": {
                  "cnr_id": "comfy-core",
                  "ver": "0.3.71",
                  "Node name for S&R": "ReferenceLatent"
                },
                "widgets_values": []
              },
              {
                "id": 50,
                "type": "PrimitiveNode",
                "pos": [-190.01410752108893, 565.9791640958119],
                "size": [252, 80],
                "flags": {},
                "order": 2,
                "mode": 0,
                "inputs": [],
                "outputs": [
                  {
                    "name": "INT",
                    "type": "INT",
                    "widget": { "name": "width" },
                    "links": [135, 136]
                  }
                ],
                "title": "width",
                "properties": { "Run widget replace on values": false },
                "widgets_values": [960, "fixed"],
                "color": "#223",
                "bgcolor": "#335"
              },
              {
                "id": 51,
                "type": "PrimitiveNode",
                "pos": [-190.01410752108893, 709.9728115892052],
                "size": [252, 80],
                "flags": {},
                "order": 3,
                "mode": 0,
                "inputs": [],
                "outputs": [
                  {
                    "name": "INT",
                    "type": "INT",
                    "widget": { "name": "height" },
                    "links": [137, 138]
                  }
                ],
                "title": "height",
                "properties": { "Run widget replace on values": false },
                "widgets_values": [1664, "fixed"],
                "color": "#223",
                "bgcolor": "#335"
              },
              {
                "id": 10,
                "type": "VAELoader",
                "pos": [-1165.8025389323398, 277.99193140321734],
                "size": [357.8166809082031, 82.66666412353516],
                "flags": {},
                "order": 4,
                "mode": 0,
                "inputs": [],
                "outputs": [
                  {
                    "name": "VAE",
                    "type": "VAE",
                    "slot_index": 0,
                    "links": [120, 127, 159]
                  }
                ],
                "properties": {
                  "cnr_id": "comfy-core",
                  "ver": "0.3.71",
                  "Node name for S&R": "VAELoader",
                  "models": [
                    {
                      "name": "flux2-vae.safetensors",
                      "url": "https://huggingface.co/Comfy-Org/flux2-dev/resolve/main/split_files/vae/flux2-vae.safetensors",
                      "directory": "vae"
                    }
                  ]
                },
                "widgets_values": ["flux2-vae.safetensors"]
              },
              {
                "id": 47,
                "type": "EmptyFlux2LatentImage",
                "pos": [133.98427620709526, 601.9733088170158],
                "size": [324, 144],
                "flags": {},
                "order": 12,
                "mode": 0,
                "inputs": [
                  {
                    "name": "width",
                    "type": "INT",
                    "widget": { "name": "width" },
                    "link": 135
                  },
                  {
                    "name": "height",
                    "type": "INT",
                    "widget": { "name": "height" },
                    "link": 137
                  }
                ],
                "outputs": [{ "name": "LATENT", "type": "LATENT", "links": [161] }],
                "properties": {
                  "cnr_id": "comfy-core",
                  "ver": "0.3.71",
                  "Node name for S&R": "EmptyFlux2LatentImage"
                },
                "widgets_values": [960, 1664, 1]
              },
              {
                "id": 13,
                "type": "SamplerCustomAdvanced",
                "pos": [109.98251095483067, -46.0064523249668],
                "size": [326.8333435058594, 144],
                "flags": {},
                "order": 23,
                "mode": 0,
                "inputs": [
                  { "name": "noise", "type": "NOISE", "link": 37 },
                  { "name": "guider", "type": "GUIDER", "link": 30 },
                  { "name": "sampler", "type": "SAMPLER", "link": 19 },
                  { "name": "sigmas", "type": "SIGMAS", "link": 132 },
                  { "name": "latent_image", "type": "LATENT", "link": 161 }
                ],
                "outputs": [
                  {
                    "name": "output",
                    "type": "LATENT",
                    "slot_index": 0,
                    "links": [24]
                  },
                  { "name": "denoised_output", "type": "LATENT", "links": null }
                ],
                "properties": {
                  "cnr_id": "comfy-core",
                  "ver": "0.3.71",
                  "Node name for S&R": "SamplerCustomAdvanced"
                },
                "widgets_values": []
              },
              {
                "id": 41,
                "type": "ImageScaleToTotalPixels",
                "pos": [-641.0976573853251, 901.9698649987437],
                "size": [324, 146.6666717529297],
                "flags": {},
                "order": 11,
                "mode": 4,
                "inputs": [{ "name": "image", "type": "IMAGE", "link": 123 }],
                "outputs": [{ "name": "IMAGE", "type": "IMAGE", "links": [122] }],
                "properties": {
                  "cnr_id": "comfy-core",
                  "ver": "0.3.71",
                  "Node name for S&R": "ImageScaleToTotalPixels"
                },
                "widgets_values": ["lanczos", 1, 1]
              },
              {
                "id": 8,
                "type": "VAEDecode",
                "pos": [-166.0123111217285, 901.9698649987437],
                "size": [252, 72],
                "flags": {},
                "order": 24,
                "mode": 0,
                "inputs": [
                  { "name": "samples", "type": "LATENT", "link": 24 },
                  { "name": "vae", "type": "VAE", "link": 159 }
                ],
                "outputs": [
                  {
                    "name": "IMAGE",
                    "type": "IMAGE",
                    "slot_index": 0,
                    "links": [9]
                  }
                ],
                "properties": {
                  "cnr_id": "comfy-core",
                  "ver": "0.3.71",
                  "Node name for S&R": "VAEDecode"
                },
                "widgets_values": []
              },
              {
                "id": 45,
                "type": "ImageScaleToTotalPixels",
                "pos": [-1162.0091964114495, 901.9698649987437],
                "size": [324, 146.6666717529297],
                "flags": {},
                "order": 14,
                "mode": 0,
                "inputs": [{ "name": "image", "type": "IMAGE", "link": 128 }],
                "outputs": [{ "name": "IMAGE", "type": "IMAGE", "links": [126] }],
                "properties": {
                  "cnr_id": "comfy-core",
                  "ver": "0.3.71",
                  "Node name for S&R": "ImageScaleToTotalPixels"
                },
                "widgets_values": ["lanczos", 1, 1]
              },
              {
                "id": 60,
                "type": "Note",
                "pos": [-1174.001575880389, 1286.0151776435532],
                "size": [552, 97.5],
                "flags": {},
                "order": 5,
                "mode": 0,
                "inputs": [],
                "outputs": [],
                "properties": {},
                "widgets_values": [
                  "Unbypass (CTRL-B) the ReferenceLatent nodes to give ref images.\n\nIf you want to add more reference images, you can add multiple reference images by following the pattern.\n\nIf you don't want any reference image, just select all ReferenceLatent nodes and then use CTRL-B to bypass them, turning the workflow into a Text to Image workflow."
                ],
                "color": "#432",
                "bgcolor": "#653"
              },
              {
                "id": 6,
                "type": "CLIPTextEncode",
                "pos": [-753.9961311431468, -57.99883179390622],
                "size": [516, 121.5],
                "flags": {},
                "order": 15,
                "mode": 0,
                "inputs": [{ "name": "clip", "type": "CLIP", "link": 117 }],
                "outputs": [
                  {
                    "name": "CONDITIONING",
                    "type": "CONDITIONING",
                    "slot_index": 0,
                    "links": [41]
                  }
                ],
                "title": "CLIP Text Encode (Positive Prompt)",
                "properties": {
                  "cnr_id": "comfy-core",
                  "ver": "0.3.71",
                  "Node name for S&R": "CLIPTextEncode"
                },
                "widgets_values": [
                  "Photorealistic satellite view from 400 kilometers (250 miles) above Earth's surface, capturing the eastern Mediterranean region centered on Jerusalem. The perspective matches actual ISS astronaut photography: the curvature of Earth is subtly visible at this altitude, with the black void of space above transitioning to the thin blue line of Earth's atmosphere at the horizon. Jerusalem appears as a distinct point on the Judaean Mountains plateau at 31.77°N, 35.22°E, elevation 760 meters (2,490 feet). The city is visible as a small but clearly defined urban area—approximately 12-15 acres of dense settlement—showing as a tan/gray cluster against the brown and tan limestone terrain of the surrounding hills. The topography is scientifically accurate: the Judaean Mountains form a central ridge with Jerusalem on the plateau, the terrain slopes dramatically eastward into the Jordan Rift Valley (visible as a dark, shadowed depression), and descends more gradually westward toward the Mediterranean coastal plain. The Mediterranean Sea appears as a vast dark blue body of water on the western horizon, approximately 60 km from Jerusalem, its coastline visible as a subtle line where land meets sea. The Dead Sea is clearly visible 35 km to the east as a distinctive turquoise-blue body of water in the deepest terrestrial depression on Earth (430 meters below sea level), creating a stark color contrast with the surrounding desert terrain. The Jordan River valley appears as a dark, vegetation-rich corridor trending north-south between Jerusalem and the Dead Sea. The terrain shows realistic geological features: exposed limestone bedrock, terraced hillsides, sparse Mediterranean vegetation (appearing as muted green patches), and the characteristic brown and tan earth tones of the Levantine landscape. Small towns and villages appear as tiny points of human settlement scattered across the region. Highway corridors show as thin lines connecting population centers. The image captures the dramatic elevation change from the coastal plains (sea level) to the mountain plateau (760m) to the Dead Sea depression (-430m). Atmospheric perspective is scientifically accurate: distant terrain shows natural haze and reduced contrast, while the foreground maintains sharp detail. The sky grades from deep black at the top of the frame through navy blue to the thin atmospheric halo at Earth's limb. Cloud formations appear realistic for the region—sparse, wispy cirrus clouds typical of the eastern Mediterranean climate. The sun angle creates scientifically accurate shadows based on time of day, emphasizing the three-dimensional topography. The overall image conveys the remarkable geographic reality: Jerusalem as a small but significant human settlement perched on a mountain plateau between the Mediterranean Sea and the Dead Sea, positioned at the crossroads of ancient civilizations yet geographically isolated by dramatic terrain."
                ],
                "color": "#232",
                "bgcolor": "#353"
              },
              {
                "id": 25,
                "type": "RandomNoise",
                "pos": [-190.01410752108893, -57.99883179390622],
                "size": [266.8166809082031, 80],
                "flags": {},
                "order": 6,
                "mode": 0,
                "inputs": [],
                "outputs": [{ "name": "NOISE", "type": "NOISE", "links": [37] }],
                "properties": {
                  "cnr_id": "comfy-core",
                  "ver": "0.3.71",
                  "Node name for S&R": "RandomNoise"
                },
                "widgets_values": [730539084012896, "randomize"]
              },
              {
                "id": 46,
                "type": "LoadImage",
                "pos": [-1161.9922212441595, 469.972040792562],
                "size": [328.8999938964844, 454.8999938964844],
                "flags": {},
                "order": 7,
                "mode": 0,
                "inputs": [],
                "outputs": [
                  { "name": "IMAGE", "type": "IMAGE", "links": [128] },
                  { "name": "MASK", "type": "MASK", "links": null }
                ],
                "properties": {
                  "cnr_id": "comfy-core",
                  "ver": "0.3.71",
                  "Node name for S&R": "LoadImage"
                },
                "widgets_values": ["example.png", "image"]
              },
              {
                "id": 9,
                "type": "SaveImage",
                "pos": [493.9768046564832, -106.00236229843523],
                "size": [506.1499938964844, 922.1500244140625],
                "flags": {},
                "order": 25,
                "mode": 0,
                "inputs": [{ "name": "images", "type": "IMAGE", "link": 9 }],
                "outputs": [],
                "properties": { "cnr_id": "comfy-core", "ver": "0.3.71" },
                "widgets_values": ["2026.02.22_flux2"]
              },
              {
                "id": 38,
                "type": "CLIPLoader",
                "pos": [-1165.8025389323398, 100.36863983015184],
                "size": [357.8166809082031, 152],
                "flags": {},
                "order": 8,
                "mode": 0,
                "inputs": [],
                "outputs": [{ "name": "CLIP", "type": "CLIP", "links": [117] }],
                "properties": {
                  "cnr_id": "comfy-core",
                  "ver": "0.3.71",
                  "Node name for S&R": "CLIPLoader",
                  "models": [
                    {
                      "name": "mistral_3_small_flux2_bf16.safetensors",
                      "url": "https://huggingface.co/Comfy-Org/flux2-dev/resolve/main/split_files/text_encoders/mistral_3_small_flux2_bf16.safetensors",
                      "directory": "text_encoders"
                    }
                  ]
                },
                "widgets_values": [
                  "mistral_3_small_flux2_bf16.safetensors",
                  "flux2",
                  "default"
                ]
              },
              {
                "id": 12,
                "type": "UNETLoader",
                "pos": [-1165.8025389323398, -48.387896986692226],
                "size": [357.8166809082031, 117.33332824707031],
                "flags": {},
                "order": 9,
                "mode": 0,
                "inputs": [],
                "outputs": [
                  {
                    "name": "MODEL",
                    "type": "MODEL",
                    "slot_index": 0,
                    "links": [164]
                  }
                ],
                "properties": {
                  "cnr_id": "comfy-core",
                  "ver": "0.3.71",
                  "Node name for S&R": "UNETLoader",
                  "models": [
                    {
                      "name": "flux2_dev_fp8mixed.safetensors",
                      "url": "https://huggingface.co/Comfy-Org/flux2-dev/resolve/main/split_files/diffusion_models/flux2_dev_fp8mixed.safetensors",
                      "directory": "diffusion_models"
                    }
                  ]
                },
                "widgets_values": ["flux2_dev_fp8mixed.safetensors", "default"]
              },
              {
                "id": 67,
                "type": "LoraLoaderModelOnly",
                "pos": [-657.9890545605408, -273.997743896997],
                "size": [324, 114.66667175292969],
                "flags": {},
                "order": 16,
                "mode": 4,
                "inputs": [{ "name": "model", "type": "MODEL", "link": 164 }],
                "outputs": [{ "name": "MODEL", "type": "MODEL", "links": [165] }],
                "properties": {
                  "cnr_id": "comfy-core",
                  "ver": "0.4.0",
                  "Node name for S&R": "LoraLoaderModelOnly",
                  "models": [
                    {
                      "name": "flux2_berthe_morisot.safetensors",
                      "url": "https://huggingface.co/ostris/flux2_berthe_morisot/resolve/main/flux2_berthe_morisot.safetensors",
                      "directory": "loras"
                    }
                  ]
                },
                "widgets_values": ["flux2_berthe_morisot.safetensors", 1]
              },
              {
                "id": 66,
                "type": "MarkdownNote",
                "pos": [-1822.015349651143, -106.00236229843523],
                "size": [624, 780],
                "flags": { "collapsed": false },
                "order": 10,
                "mode": 0,
                "inputs": [],
                "outputs": [],
                "title": "Model links",
                "properties": {},
                "widgets_values": [
                  "We are using quantized weights in this workflow, the original flux 2 repo is [here](https://huggingface.co/black-forest-labs/FLUX.2-dev/)\n## Report issue\n\nIf you found any issues when running this workflow, [report template issue here](https://github.com/Comfy-Org/workflow_templates/issues)\n\n\n## Model links\n\n**text_encoders**\n\n- [mistral_3_small_flux2_bf16.safetensors](https://huggingface.co/Comfy-Org/flux2-dev/resolve/main/split_files/text_encoders/mistral_3_small_flux2_bf16.safetensors)\n\n**loras**\n\n- [flux2_berthe_morisot.safetensors](https://huggingface.co/ostris/flux2_berthe_morisot/resolve/main/flux2_berthe_morisot.safetensors)\n\n**diffusion_models**\n\n- [flux2_dev_fp8mixed.safetensors](https://huggingface.co/Comfy-Org/flux2-dev/resolve/main/split_files/diffusion_models/flux2_dev_fp8mixed.safetensors)\n\n**vae**\n\n- [flux2-vae.safetensors](https://huggingface.co/Comfy-Org/flux2-dev/resolve/main/split_files/vae/flux2-vae.safetensors)\n\n\nModel Storage Location\n\n```\n📂 ComfyUI/\n├── 📂 models/\n│   ├── 📂 text_encoders/\n│   │      └── mistral_3_small_flux2_bf16.safetensors\n│   ├── 📂 loras/\n│   │      └── flux2_berthe_morisot.safetensors\n│   ├── 📂 diffusion_models/\n│   │      └── flux2_dev_fp8mixed.safetensors\n│   └── 📂 vae/\n│          └── flux2-vae.safetensors\n```\n"
                ],
                "color": "#432",
                "bgcolor": "#653"
              }
            ],
            "links": [
              [9, 8, 0, 9, 0, "IMAGE"],
              [19, 16, 0, 13, 2, "SAMPLER"],
              [24, 13, 0, 8, 0, "LATENT"],
              [30, 22, 0, 13, 1, "GUIDER"],
              [37, 25, 0, 13, 0, "NOISE"],
              [41, 6, 0, 26, 0, "CONDITIONING"],
              [117, 38, 0, 6, 0, "CLIP"],
              [120, 10, 0, 40, 1, "VAE"],
              [121, 40, 0, 39, 1, "LATENT"],
              [122, 41, 0, 40, 0, "IMAGE"],
              [123, 42, 0, 41, 0, "IMAGE"],
              [125, 44, 0, 43, 1, "LATENT"],
              [126, 45, 0, 44, 0, "IMAGE"],
              [127, 10, 0, 44, 1, "VAE"],
              [128, 46, 0, 45, 0, "IMAGE"],
              [132, 48, 0, 13, 3, "SIGMAS"],
              [135, 50, 0, 47, 0, "INT"],
              [136, 50, 0, 48, 0, "INT"],
              [137, 51, 0, 47, 1, "INT"],
              [138, 51, 0, 48, 1, "INT"],
              [144, 26, 0, 43, 0, "CONDITIONING"],
              [145, 43, 0, 39, 0, "CONDITIONING"],
              [153, 39, 0, 22, 1, "CONDITIONING"],
              [159, 10, 0, 8, 1, "VAE"],
              [161, 47, 0, 13, 4, "LATENT"],
              [164, 12, 0, 67, 0, "MODEL"],
              [165, 67, 0, 22, 0, "MODEL"]
            ],
            "groups": [
              {
                "id": 1,
                "title": "Step 1 - Upload models",
                "bounding": [-1174, -142, 381.81818181818176, 499.23588134765623],
                "color": "#3f789e",
                "font_size": 24,
                "flags": {}
              },
              {
                "id": 2,
                "title": "Custom sampler",
                "bounding": [-202, -142, 670.2431030273438, 601.92],
                "color": "#3f789e",
                "font_size": 24,
                "flags": {}
              },
              {
                "id": 3,
                "title": "Step 4- Image size",
                "bounding": [-202, 482, 672, 342.72],
                "color": "#3f789e",
                "font_size": 24,
                "flags": {}
              },
              {
                "id": 4,
                "title": "Step2 - Prompt",
                "bounding": [-766, -142, 540, 504],
                "color": "#3f789e",
                "font_size": 24,
                "flags": {}
              },
              {
                "id": 5,
                "title": "Step 3- Reference images",
                "bounding": [-1174, 386, 415.73227090667297, 837.5300801434184],
                "color": "#3f789e",
                "font_size": 24,
                "flags": {}
              },
              {
                "id": 6,
                "title": "Ctrl-B to Enable",
                "bounding": [-766, -370, 540, 204],
                "color": "#3f789e",
                "font_size": 24,
                "flags": {}
              }
            ],
            "config": {},
            "extra": {
              "ds": {
                "scale": 0.3919155503854037,
                "offset": [2764.3782127553486, 1094.6272487836547]
              },
              "frontendVersion": "1.37.11",
              "workflowRendererVersion": "Vue",
              "groupNodes": {},
              "VHS_latentpreview": false,
              "VHS_latentpreviewrate": 0,
              "VHS_MetadataImage": true,
              "VHS_KeepIntermediate": true
            },
            "version": 0.4
          }
        }
      }
    }
  image_flux2_text_landscape.json: |
    {
      "client_id": "70b193468f844b99ada3c47dd94e287e",
      "prompt": {
        "6": {
          "inputs": {
            "text": "Photorealistic satellite view from 400 kilometers (250 miles) above Earth's surface, capturing the eastern Mediterranean region centered on Jerusalem. The perspective matches actual ISS astronaut photography: the curvature of Earth is subtly visible at this altitude, with the black void of space above transitioning to the thin blue line of Earth's atmosphere at the horizon. Jerusalem appears as a distinct point on the Judaean Mountains plateau at 31.77°N, 35.22°E, elevation 760 meters (2,490 feet). The city is visible as a small but clearly defined urban area—approximately 12-15 acres of dense settlement—showing as a tan/gray cluster against the brown and tan limestone terrain of the surrounding hills. The topography is scientifically accurate: the Judaean Mountains form a central ridge with Jerusalem on the plateau, the terrain slopes dramatically eastward into the Jordan Rift Valley (visible as a dark, shadowed depression), and descends more gradually westward toward the Mediterranean coastal plain. The Mediterranean Sea appears as a vast dark blue body of water on the western horizon, approximately 60 km from Jerusalem, its coastline visible as a subtle line where land meets sea. The Dead Sea is clearly visible 35 km to the east as a distinctive turquoise-blue body of water in the deepest terrestrial depression on Earth (430 meters below sea level), creating a stark color contrast with the surrounding desert terrain. The Jordan River valley appears as a dark, vegetation-rich corridor trending north-south between Jerusalem and the Dead Sea. The terrain shows realistic geological features: exposed limestone bedrock, terraced hillsides, sparse Mediterranean vegetation (appearing as muted green patches), and the characteristic brown and tan earth tones of the Levantine landscape. Small towns and villages appear as tiny points of human settlement scattered across the region. Highway corridors show as thin lines connecting population centers. The image captures the dramatic elevation change from the coastal plains (sea level) to the mountain plateau (760m) to the Dead Sea depression (-430m). Atmospheric perspective is scientifically accurate: distant terrain shows natural haze and reduced contrast, while the foreground maintains sharp detail. The sky grades from deep black at the top of the frame through navy blue to the thin atmospheric halo at Earth's limb. Cloud formations appear realistic for the region—sparse, wispy cirrus clouds typical of the eastern Mediterranean climate. The sun angle creates scientifically accurate shadows based on time of day, emphasizing the three-dimensional topography. The overall image conveys the remarkable geographic reality: Jerusalem as a small but significant human settlement perched on a mountain plateau between the Mediterranean Sea and the Dead Sea, positioned at the crossroads of ancient civilizations yet geographically isolated by dramatic terrain.",
            "clip": ["38", 0]
          },
          "class_type": "CLIPTextEncode",
          "_meta": { "title": "CLIP Text Encode (Positive Prompt)" }
        },
        "8": {
          "inputs": { "samples": ["13", 0], "vae": ["10", 0] },
          "class_type": "VAEDecode",
          "_meta": { "title": "VAE Decode" }
        },
        "9": {
          "inputs": { "filename_prefix": "2026.02.22_flux2", "images": ["8", 0] },
          "class_type": "SaveImage",
          "_meta": { "title": "Save Image" }
        },
        "10": {
          "inputs": { "vae_name": "flux2-vae.safetensors" },
          "class_type": "VAELoader",
          "_meta": { "title": "Load VAE" }
        },
        "12": {
          "inputs": {
            "unet_name": "flux2_dev_fp8mixed.safetensors",
            "weight_dtype": "default"
          },
          "class_type": "UNETLoader",
          "_meta": { "title": "Load Diffusion Model" }
        },
        "13": {
          "inputs": {
            "noise": ["25", 0],
            "guider": ["22", 0],
            "sampler": ["16", 0],
            "sigmas": ["48", 0],
            "latent_image": ["47", 0]
          },
          "class_type": "SamplerCustomAdvanced",
          "_meta": { "title": "SamplerCustomAdvanced" }
        },
        "16": {
          "inputs": { "sampler_name": "euler" },
          "class_type": "KSamplerSelect",
          "_meta": { "title": "KSamplerSelect" }
        },
        "22": {
          "inputs": { "model": ["12", 0], "conditioning": ["26", 0] },
          "class_type": "BasicGuider",
          "_meta": { "title": "BasicGuider" }
        },
        "25": {
          "inputs": { "noise_seed": 730539084012896 },
          "class_type": "RandomNoise",
          "_meta": { "title": "RandomNoise" }
        },
        "26": {
          "inputs": { "guidance": 4, "conditioning": ["6", 0] },
          "class_type": "FluxGuidance",
          "_meta": { "title": "FluxGuidance" }
        },
        "38": {
          "inputs": {
            "clip_name": "mistral_3_small_flux2_bf16.safetensors",
            "type": "flux2",
            "device": "default"
          },
          "class_type": "CLIPLoader",
          "_meta": { "title": "Load CLIP" }
        },
        "47": {
          "inputs": { "width": 1248, "height": 832, "batch_size": 1 },
          "class_type": "EmptyFlux2LatentImage",
          "_meta": { "title": "Empty Flux 2 Latent" }
        },
        "48": {
          "inputs": { "steps": 20, "width": 1248, "height": 832 },
          "class_type": "Flux2Scheduler",
          "_meta": { "title": "Flux2Scheduler" }
        }
      },
      "extra_data": {
        "extra_pnginfo": {
          "workflow": {
            "id": "7c048efb-a059-44e2-970a-43e1eb472d0d",
            "revision": 0,
            "last_node_id": 67,
            "last_link_id": 165,
            "nodes": [
              {
                "id": 48,
                "type": "Flux2Scheduler",
                "pos": [-190.01617800988822, 313.9950760488025],
                "size": [266.8166809082031, 144],
                "flags": {},
                "order": 13,
                "mode": 0,
                "inputs": [
                  {
                    "name": "width",
                    "type": "INT",
                    "widget": { "name": "width" },
                    "link": 136
                  },
                  {
                    "name": "height",
                    "type": "INT",
                    "widget": { "name": "height" },
                    "link": 138
                  }
                ],
                "outputs": [{ "name": "SIGMAS", "type": "SIGMAS", "links": [132] }],
                "properties": {
                  "cnr_id": "comfy-core",
                  "ver": "0.3.71",
                  "Node name for S&R": "Flux2Scheduler"
                },
                "widgets_values": [20, 1248, 832]
              },
              {
                "id": 22,
                "type": "BasicGuider",
                "pos": [-190.01617800988822, 85.9867470152908],
                "size": [266.8166809082031, 72],
                "flags": {},
                "order": 22,
                "mode": 0,
                "inputs": [
                  { "name": "model", "type": "MODEL", "link": 165 },
                  { "name": "conditioning", "type": "CONDITIONING", "link": 153 }
                ],
                "outputs": [
                  {
                    "name": "GUIDER",
                    "type": "GUIDER",
                    "slot_index": 0,
                    "links": [30]
                  }
                ],
                "properties": {
                  "cnr_id": "comfy-core",
                  "ver": "0.3.71",
                  "Node name for S&R": "BasicGuider"
                },
                "widgets_values": []
              },
              {
                "id": 40,
                "type": "VAEEncode",
                "pos": [-485.11379435307936, 1058.0158485344223],
                "size": [228.75, 8],
                "flags": { "collapsed": true },
                "order": 17,
                "mode": 4,
                "inputs": [
                  { "name": "pixels", "type": "IMAGE", "link": 122 },
                  { "name": "vae", "type": "VAE", "link": 120 }
                ],
                "outputs": [{ "name": "LATENT", "type": "LATENT", "links": [121] }],
                "properties": {
                  "cnr_id": "comfy-core",
                  "ver": "0.3.71",
                  "Node name for S&R": "VAEEncode"
                },
                "widgets_values": []
              },
              {
                "id": 42,
                "type": "LoadImage",
                "pos": [-645.9987767274963, 481.9904265002683],
                "size": [328.8999938964844, 344],
                "flags": {},
                "order": 0,
                "mode": 4,
                "inputs": [],
                "outputs": [
                  { "name": "IMAGE", "type": "IMAGE", "links": [123] },
                  { "name": "MASK", "type": "MASK", "links": null }
                ],
                "properties": {
                  "cnr_id": "comfy-core",
                  "ver": "0.3.71",
                  "Node name for S&R": "LoadImage"
                },
                "widgets_values": ["image_flux2_input_image.png", "image"]
              },
              {
                "id": 44,
                "type": "VAEEncode",
                "pos": [-1005.9912896033362, 1058.0158485344223],
                "size": [228.75, 8],
                "flags": { "collapsed": true },
                "order": 18,
                "mode": 4,
                "inputs": [
                  { "name": "pixels", "type": "IMAGE", "link": 126 },
                  { "name": "vae", "type": "VAE", "link": 127 }
                ],
                "outputs": [{ "name": "LATENT", "type": "LATENT", "links": [125] }],
                "properties": {
                  "cnr_id": "comfy-core",
                  "ver": "0.3.71",
                  "Node name for S&R": "VAEEncode"
                },
                "widgets_values": []
              },
              {
                "id": 39,
                "type": "ReferenceLatent",
                "pos": [-554.3804038312119, 1106.0024350187573],
                "size": [237.25, 72],
                "flags": {},
                "order": 21,
                "mode": 4,
                "inputs": [
                  { "name": "conditioning", "type": "CONDITIONING", "link": 145 },
                  { "name": "latent", "shape": 7, "type": "LATENT", "link": 121 }
                ],
                "outputs": [
                  { "name": "CONDITIONING", "type": "CONDITIONING", "links": [153] }
                ],
                "properties": {
                  "cnr_id": "comfy-core",
                  "ver": "0.3.71",
                  "Node name for S&R": "ReferenceLatent"
                },
                "widgets_values": []
              },
              {
                "id": 26,
                "type": "FluxGuidance",
                "pos": [-621.9970114752318, 265.99154554427355],
                "size": [380.8833312988281, 80],
                "flags": {},
                "order": 19,
                "mode": 0,
                "inputs": [
                  { "name": "conditioning", "type": "CONDITIONING", "link": 41 }
                ],
                "outputs": [
                  {
                    "name": "CONDITIONING",
                    "type": "CONDITIONING",
                    "slot_index": 0,
                    "links": [144]
                  }
                ],
                "properties": {
                  "cnr_id": "comfy-core",
                  "ver": "0.3.71",
                  "Node name for S&R": "FluxGuidance"
                },
                "widgets_values": [4],
                "color": "#233",
                "bgcolor": "#355"
              },
              {
                "id": 16,
                "type": "KSamplerSelect",
                "pos": [-190.01617800988822, 193.98618749328818],
                "size": [266.8166809082031, 82.66666412353516],
                "flags": {},
                "order": 1,
                "mode": 0,
                "inputs": [],
                "outputs": [
                  { "name": "SAMPLER", "type": "SAMPLER", "links": [19] }
                ],
                "properties": {
                  "cnr_id": "comfy-core",
                  "ver": "0.3.71",
                  "Node name for S&R": "KSamplerSelect"
                },
                "widgets_values": ["euler"]
              },
              {
                "id": 43,
                "type": "ReferenceLatent",
                "pos": [-1089.9889648290691, 1106.0024350187573],
                "size": [237.25, 72],
                "flags": {},
                "order": 20,
                "mode": 4,
                "inputs": [
                  { "name": "conditioning", "type": "CONDITIONING", "link": 144 },
                  { "name": "latent", "shape": 7, "type": "LATENT", "link": 125 }
                ],
                "outputs": [
                  { "name": "CONDITIONING", "type": "CONDITIONING", "links": [145] }
                ],
                "properties": {
                  "cnr_id": "comfy-core",
                  "ver": "0.3.71",
                  "Node name for S&R": "ReferenceLatent"
                },
                "widgets_values": []
              },
              {
                "id": 50,
                "type": "PrimitiveNode",
                "pos": [-190.01617800988822, 565.9710954116155],
                "size": [252, 80],
                "flags": {},
                "order": 2,
                "mode": 0,
                "inputs": [],
                "outputs": [
                  {
                    "name": "INT",
                    "type": "INT",
                    "widget": { "name": "width" },
                    "links": [135, 136]
                  }
                ],
                "title": "width",
                "properties": { "Run widget replace on values": false },
                "widgets_values": [1248, "fixed"],
                "color": "#223",
                "bgcolor": "#335"
              },
              {
                "id": 51,
                "type": "PrimitiveNode",
                "pos": [-190.01617800988822, 709.9816869252023],
                "size": [252, 80],
                "flags": {},
                "order": 3,
                "mode": 0,
                "inputs": [],
                "outputs": [
                  {
                    "name": "INT",
                    "type": "INT",
                    "widget": { "name": "height" },
                    "links": [137, 138]
                  }
                ],
                "title": "height",
                "properties": { "Run widget replace on values": false },
                "widgets_values": [832, "fixed"],
                "color": "#223",
                "bgcolor": "#335"
              },
              {
                "id": 10,
                "type": "VAELoader",
                "pos": [-1165.8046405682344, 277.9839250132129],
                "size": [357.8166809082031, 82.66666412353516],
                "flags": {},
                "order": 4,
                "mode": 0,
                "inputs": [],
                "outputs": [
                  {
                    "name": "VAE",
                    "type": "VAE",
                    "slot_index": 0,
                    "links": [120, 127, 159]
                  }
                ],
                "properties": {
                  "cnr_id": "comfy-core",
                  "ver": "0.3.71",
                  "Node name for S&R": "VAELoader",
                  "models": [
                    {
                      "name": "flux2-vae.safetensors",
                      "url": "https://huggingface.co/Comfy-Org/flux2-dev/resolve/main/split_files/vae/flux2-vae.safetensors",
                      "directory": "vae"
                    }
                  ]
                },
                "widgets_values": ["flux2-vae.safetensors"]
              },
              {
                "id": 47,
                "type": "EmptyFlux2LatentImage",
                "pos": [133.98220571829575, 601.9822464472051],
                "size": [324, 144],
                "flags": {},
                "order": 12,
                "mode": 0,
                "inputs": [
                  {
                    "name": "width",
                    "type": "INT",
                    "widget": { "name": "width" },
                    "link": 135
                  },
                  {
                    "name": "height",
                    "type": "INT",
                    "widget": { "name": "height" },
                    "link": 137
                  }
                ],
                "outputs": [{ "name": "LATENT", "type": "LATENT", "links": [161] }],
                "properties": {
                  "cnr_id": "comfy-core",
                  "ver": "0.3.71",
                  "Node name for S&R": "EmptyFlux2LatentImage"
                },
                "widgets_values": [1248, 832, 1]
              },
              {
                "id": 13,
                "type": "SamplerCustomAdvanced",
                "pos": [109.99744678041702, -45.99745240058536],
                "size": [326.8333435058594, 144],
                "flags": {},
                "order": 23,
                "mode": 0,
                "inputs": [
                  { "name": "noise", "type": "NOISE", "link": 37 },
                  { "name": "guider", "type": "GUIDER", "link": 30 },
                  { "name": "sampler", "type": "SAMPLER", "link": 19 },
                  { "name": "sigmas", "type": "SIGMAS", "link": 132 },
                  { "name": "latent_image", "type": "LATENT", "link": 161 }
                ],
                "outputs": [
                  {
                    "name": "output",
                    "type": "LATENT",
                    "slot_index": 0,
                    "links": [24]
                  },
                  { "name": "denoised_output", "type": "LATENT", "links": null }
                ],
                "properties": {
                  "cnr_id": "comfy-core",
                  "ver": "0.3.71",
                  "Node name for S&R": "SamplerCustomAdvanced"
                },
                "widgets_values": []
              },
              {
                "id": 41,
                "type": "ImageScaleToTotalPixels",
                "pos": [-641.09975902122, 901.9788649231244],
                "size": [324, 146.6666717529297],
                "flags": {},
                "order": 11,
                "mode": 4,
                "inputs": [{ "name": "image", "type": "IMAGE", "link": 123 }],
                "outputs": [{ "name": "IMAGE", "type": "IMAGE", "links": [122] }],
                "properties": {
                  "cnr_id": "comfy-core",
                  "ver": "0.3.71",
                  "Node name for S&R": "ImageScaleToTotalPixels"
                },
                "widgets_values": ["lanczos", 1, 1]
              },
              {
                "id": 8,
                "type": "VAEDecode",
                "pos": [-166.01441275762375, 901.9788649231244],
                "size": [252, 72],
                "flags": {},
                "order": 24,
                "mode": 0,
                "inputs": [
                  { "name": "samples", "type": "LATENT", "link": 24 },
                  { "name": "vae", "type": "VAE", "link": 159 }
                ],
                "outputs": [
                  {
                    "name": "IMAGE",
                    "type": "IMAGE",
                    "slot_index": 0,
                    "links": [9]
                  }
                ],
                "properties": {
                  "cnr_id": "comfy-core",
                  "ver": "0.3.71",
                  "Node name for S&R": "VAEDecode"
                },
                "widgets_values": []
              },
              {
                "id": 45,
                "type": "ImageScaleToTotalPixels",
                "pos": [-1162.011298047344, 901.9788649231244],
                "size": [324, 146.6666717529297],
                "flags": {},
                "order": 14,
                "mode": 4,
                "inputs": [{ "name": "image", "type": "IMAGE", "link": 128 }],
                "outputs": [{ "name": "IMAGE", "type": "IMAGE", "links": [126] }],
                "properties": {
                  "cnr_id": "comfy-core",
                  "ver": "0.3.71",
                  "Node name for S&R": "ImageScaleToTotalPixels"
                },
                "widgets_values": ["lanczos", 1, 1]
              },
              {
                "id": 60,
                "type": "Note",
                "pos": [-1174.0036775162837, 1286.0072335477403],
                "size": [552, 97.5],
                "flags": {},
                "order": 5,
                "mode": 0,
                "inputs": [],
                "outputs": [],
                "properties": {},
                "widgets_values": [
                  "Unbypass (CTRL-B) the ReferenceLatent nodes to give ref images.\n\nIf you want to add more reference images, you can add multiple reference images by following the pattern.\n\nIf you don't want any reference image, just select all ReferenceLatent nodes and then use CTRL-B to bypass them, turning the workflow into a Text to Image workflow."
                ],
                "color": "#432",
                "bgcolor": "#653"
              },
              {
                "id": 6,
                "type": "CLIPTextEncode",
                "pos": [-753.981226464656, -58.006838183910475],
                "size": [529.2666625976562, 226.83334350585938],
                "flags": {},
                "order": 15,
                "mode": 0,
                "inputs": [{ "name": "clip", "type": "CLIP", "link": 117 }],
                "outputs": [
                  {
                    "name": "CONDITIONING",
                    "type": "CONDITIONING",
                    "slot_index": 0,
                    "links": [41]
                  }
                ],
                "title": "CLIP Text Encode (Positive Prompt)",
                "properties": {
                  "cnr_id": "comfy-core",
                  "ver": "0.3.71",
                  "Node name for S&R": "CLIPTextEncode"
                },
                "widgets_values": [
                  "Photorealistic satellite view from 400 kilometers (250 miles) above Earth's surface, capturing the eastern Mediterranean region centered on Jerusalem. The perspective matches actual ISS astronaut photography: the curvature of Earth is subtly visible at this altitude, with the black void of space above transitioning to the thin blue line of Earth's atmosphere at the horizon. Jerusalem appears as a distinct point on the Judaean Mountains plateau at 31.77°N, 35.22°E, elevation 760 meters (2,490 feet). The city is visible as a small but clearly defined urban area—approximately 12-15 acres of dense settlement—showing as a tan/gray cluster against the brown and tan limestone terrain of the surrounding hills. The topography is scientifically accurate: the Judaean Mountains form a central ridge with Jerusalem on the plateau, the terrain slopes dramatically eastward into the Jordan Rift Valley (visible as a dark, shadowed depression), and descends more gradually westward toward the Mediterranean coastal plain. The Mediterranean Sea appears as a vast dark blue body of water on the western horizon, approximately 60 km from Jerusalem, its coastline visible as a subtle line where land meets sea. The Dead Sea is clearly visible 35 km to the east as a distinctive turquoise-blue body of water in the deepest terrestrial depression on Earth (430 meters below sea level), creating a stark color contrast with the surrounding desert terrain. The Jordan River valley appears as a dark, vegetation-rich corridor trending north-south between Jerusalem and the Dead Sea. The terrain shows realistic geological features: exposed limestone bedrock, terraced hillsides, sparse Mediterranean vegetation (appearing as muted green patches), and the characteristic brown and tan earth tones of the Levantine landscape. Small towns and villages appear as tiny points of human settlement scattered across the region. Highway corridors show as thin lines connecting population centers. The image captures the dramatic elevation change from the coastal plains (sea level) to the mountain plateau (760m) to the Dead Sea depression (-430m). Atmospheric perspective is scientifically accurate: distant terrain shows natural haze and reduced contrast, while the foreground maintains sharp detail. The sky grades from deep black at the top of the frame through navy blue to the thin atmospheric halo at Earth's limb. Cloud formations appear realistic for the region—sparse, wispy cirrus clouds typical of the eastern Mediterranean climate. The sun angle creates scientifically accurate shadows based on time of day, emphasizing the three-dimensional topography. The overall image conveys the remarkable geographic reality: Jerusalem as a small but significant human settlement perched on a mountain plateau between the Mediterranean Sea and the Dead Sea, positioned at the crossroads of ancient civilizations yet geographically isolated by dramatic terrain."
                ],
                "color": "#232",
                "bgcolor": "#353"
              },
              {
                "id": 25,
                "type": "RandomNoise",
                "pos": [-190.01617800988822, -58.006838183910475],
                "size": [266.8166809082031, 80],
                "flags": {},
                "order": 6,
                "mode": 0,
                "inputs": [],
                "outputs": [{ "name": "NOISE", "type": "NOISE", "links": [37] }],
                "properties": {
                  "cnr_id": "comfy-core",
                  "ver": "0.3.71",
                  "Node name for S&R": "RandomNoise"
                },
                "widgets_values": [730539084012896, "randomize"]
              },
              {
                "id": 46,
                "type": "LoadImage",
                "pos": [-1162.011298047344, 469.9810407169432],
                "size": [328.8999938964844, 344],
                "flags": {},
                "order": 7,
                "mode": 4,
                "inputs": [],
                "outputs": [
                  { "name": "IMAGE", "type": "IMAGE", "links": [128] },
                  { "name": "MASK", "type": "MASK", "links": null }
                ],
                "properties": {
                  "cnr_id": "comfy-core",
                  "ver": "0.3.71",
                  "Node name for S&R": "LoadImage"
                },
                "widgets_values": ["image_flux2_input_image.png", "image"]
              },
              {
                "id": 9,
                "type": "SaveImage",
                "pos": [493.97467187349184, -106.01036868843943],
                "size": [506.1499938964844, 862.1500244140625],
                "flags": {},
                "order": 25,
                "mode": 0,
                "inputs": [{ "name": "images", "type": "IMAGE", "link": 9 }],
                "outputs": [],
                "properties": { "cnr_id": "comfy-core", "ver": "0.3.71" },
                "widgets_values": ["2026.02.22_flux2"]
              },
              {
                "id": 38,
                "type": "CLIPLoader",
                "pos": [-1165.8046405682344, 100.37763975453322],
                "size": [357.8166809082031, 152],
                "flags": {},
                "order": 8,
                "mode": 0,
                "inputs": [],
                "outputs": [{ "name": "CLIP", "type": "CLIP", "links": [117] }],
                "properties": {
                  "cnr_id": "comfy-core",
                  "ver": "0.3.71",
                  "Node name for S&R": "CLIPLoader",
                  "models": [
                    {
                      "name": "mistral_3_small_flux2_bf16.safetensors",
                      "url": "https://huggingface.co/Comfy-Org/flux2-dev/resolve/main/split_files/text_encoders/mistral_3_small_flux2_bf16.safetensors",
                      "directory": "text_encoders"
                    }
                  ]
                },
                "widgets_values": [
                  "mistral_3_small_flux2_bf16.safetensors",
                  "flux2",
                  "default"
                ]
              },
              {
                "id": 12,
                "type": "UNETLoader",
                "pos": [-1165.8046405682344, -48.378897062310784],
                "size": [357.8166809082031, 117.33332824707031],
                "flags": {},
                "order": 9,
                "mode": 0,
                "inputs": [],
                "outputs": [
                  {
                    "name": "MODEL",
                    "type": "MODEL",
                    "slot_index": 0,
                    "links": [164]
                  }
                ],
                "properties": {
                  "cnr_id": "comfy-core",
                  "ver": "0.3.71",
                  "Node name for S&R": "UNETLoader",
                  "models": [
                    {
                      "name": "flux2_dev_fp8mixed.safetensors",
                      "url": "https://huggingface.co/Comfy-Org/flux2-dev/resolve/main/split_files/diffusion_models/flux2_dev_fp8mixed.safetensors",
                      "directory": "diffusion_models"
                    }
                  ]
                },
                "widgets_values": ["flux2_dev_fp8mixed.safetensors", "default"]
              },
              {
                "id": 67,
                "type": "LoraLoaderModelOnly",
                "pos": [-657.9911561964357, -274.0057502870011],
                "size": [324, 114.66667175292969],
                "flags": {},
                "order": 16,
                "mode": 4,
                "inputs": [{ "name": "model", "type": "MODEL", "link": 164 }],
                "outputs": [{ "name": "MODEL", "type": "MODEL", "links": [165] }],
                "properties": {
                  "cnr_id": "comfy-core",
                  "ver": "0.4.0",
                  "Node name for S&R": "LoraLoaderModelOnly",
                  "models": [
                    {
                      "name": "flux2_berthe_morisot.safetensors",
                      "url": "https://huggingface.co/ostris/flux2_berthe_morisot/resolve/main/flux2_berthe_morisot.safetensors",
                      "directory": "loras"
                    }
                  ]
                },
                "widgets_values": ["flux2_berthe_morisot.safetensors", 1]
              },
              {
                "id": 66,
                "type": "MarkdownNote",
                "pos": [-1822.0004138255558, -106.01036868843943],
                "size": [624, 780],
                "flags": { "collapsed": false },
                "order": 10,
                "mode": 0,
                "inputs": [],
                "outputs": [],
                "title": "Model links",
                "properties": {},
                "widgets_values": [
                  "We are using quantized weights in this workflow, the original flux 2 repo is [here](https://huggingface.co/black-forest-labs/FLUX.2-dev/)\n## Report issue\n\nIf you found any issues when running this workflow, [report template issue here](https://github.com/Comfy-Org/workflow_templates/issues)\n\n\n## Model links\n\n**text_encoders**\n\n- [mistral_3_small_flux2_bf16.safetensors](https://huggingface.co/Comfy-Org/flux2-dev/resolve/main/split_files/text_encoders/mistral_3_small_flux2_bf16.safetensors)\n\n**loras**\n\n- [flux2_berthe_morisot.safetensors](https://huggingface.co/ostris/flux2_berthe_morisot/resolve/main/flux2_berthe_morisot.safetensors)\n\n**diffusion_models**\n\n- [flux2_dev_fp8mixed.safetensors](https://huggingface.co/Comfy-Org/flux2-dev/resolve/main/split_files/diffusion_models/flux2_dev_fp8mixed.safetensors)\n\n**vae**\n\n- [flux2-vae.safetensors](https://huggingface.co/Comfy-Org/flux2-dev/resolve/main/split_files/vae/flux2-vae.safetensors)\n\n\nModel Storage Location\n\n```\n📂 ComfyUI/\n├── 📂 models/\n│   ├── 📂 text_encoders/\n│   │      └── mistral_3_small_flux2_bf16.safetensors\n│   ├── 📂 loras/\n│   │      └── flux2_berthe_morisot.safetensors\n│   ├── 📂 diffusion_models/\n│   │      └── flux2_dev_fp8mixed.safetensors\n│   └── 📂 vae/\n│          └── flux2-vae.safetensors\n```\n"
                ],
                "color": "#432",
                "bgcolor": "#653"
              }
            ],
            "links": [
              [9, 8, 0, 9, 0, "IMAGE"],
              [19, 16, 0, 13, 2, "SAMPLER"],
              [24, 13, 0, 8, 0, "LATENT"],
              [30, 22, 0, 13, 1, "GUIDER"],
              [37, 25, 0, 13, 0, "NOISE"],
              [41, 6, 0, 26, 0, "CONDITIONING"],
              [117, 38, 0, 6, 0, "CLIP"],
              [120, 10, 0, 40, 1, "VAE"],
              [121, 40, 0, 39, 1, "LATENT"],
              [122, 41, 0, 40, 0, "IMAGE"],
              [123, 42, 0, 41, 0, "IMAGE"],
              [125, 44, 0, 43, 1, "LATENT"],
              [126, 45, 0, 44, 0, "IMAGE"],
              [127, 10, 0, 44, 1, "VAE"],
              [128, 46, 0, 45, 0, "IMAGE"],
              [132, 48, 0, 13, 3, "SIGMAS"],
              [135, 50, 0, 47, 0, "INT"],
              [136, 50, 0, 48, 0, "INT"],
              [137, 51, 0, 47, 1, "INT"],
              [138, 51, 0, 48, 1, "INT"],
              [144, 26, 0, 43, 0, "CONDITIONING"],
              [145, 43, 0, 39, 0, "CONDITIONING"],
              [153, 39, 0, 22, 1, "CONDITIONING"],
              [159, 10, 0, 8, 1, "VAE"],
              [161, 47, 0, 13, 4, "LATENT"],
              [164, 12, 0, 67, 0, "MODEL"],
              [165, 67, 0, 22, 0, "MODEL"]
            ],
            "groups": [
              {
                "id": 1,
                "title": "Step 1 - Upload models",
                "bounding": [-1174, -142, 381.81818181818176, 499.23588134765623],
                "color": "#3f789e",
                "font_size": 24,
                "flags": {}
              },
              {
                "id": 2,
                "title": "Custom sampler",
                "bounding": [-202, -142, 670.2431030273438, 601.92],
                "color": "#3f789e",
                "font_size": 24,
                "flags": {}
              },
              {
                "id": 3,
                "title": "Step 4- Image size",
                "bounding": [-202, 482, 672, 342.72],
                "color": "#3f789e",
                "font_size": 24,
                "flags": {}
              },
              {
                "id": 4,
                "title": "Step2 - Prompt",
                "bounding": [-766, -142, 540, 504],
                "color": "#3f789e",
                "font_size": 24,
                "flags": {}
              },
              {
                "id": 5,
                "title": "Step 3- Reference images",
                "bounding": [-1174, 386, 415.73227090667297, 837.5300801434184],
                "color": "#3f789e",
                "font_size": 24,
                "flags": {}
              },
              {
                "id": 6,
                "title": "Ctrl-B to Enable",
                "bounding": [-766, -370, 540, 204],
                "color": "#3f789e",
                "font_size": 24,
                "flags": {}
              }
            ],
            "config": {},
            "extra": {
              "ds": {
                "scale": 0.9797888759635097,
                "offset": [1586.9667950075677, 632.4948911613695]
              },
              "frontendVersion": "1.37.11",
              "workflowRendererVersion": "Vue",
              "groupNodes": {},
              "VHS_latentpreview": false,
              "VHS_latentpreviewrate": 0,
              "VHS_MetadataImage": true,
              "VHS_KeepIntermediate": true
            },
            "version": 0.4
          }
        }
      }
    }
  precheck.sh: |
    #!/usr/bin/env bash
    # comfy's root onentry: fail fast if a tool the scripts need is missing.
    set -euo pipefail
    for tool in curl jq; do
      command -v "$tool" >/dev/null || { echo "$tool not found" >&2; exit 1; }
    done
    echo "tools ok"
  submit.sh: |
    #!/usr/bin/env bash
    # comfy's submit: upload input_image, if any, and point every LoadImage node at
    # it (POST /upload/image); queue the payload (POST /prompt), and keep the
    # prompt id in runs/<id>/comfy-prompt-id for await. Safe to repeat: the upload
    # overwrites, and a repeat queues the same payload.
    set -euo pipefail
    COMFY_URL="${COMFY_URL:-http://127.0.0.1:8188}"
    payload="${DECREE_RUN_DIR}/comfy-payload.json"

    if [ -n "${DECREE_DATA_INPUT_IMAGE}" ]; then
      uploaded=$(curl -fsS --max-time 60 -F "image=@${DECREE_PROJECT_ROOT}/${DECREE_DATA_INPUT_IMAGE}" \
        -F overwrite=true "${COMFY_URL}/upload/image")
      echo "uploaded: ${uploaded}"
      # ComfyUI replies {"name", "subfolder", "type": "input"}; LoadImage takes subfolder/name.
      image=$(jq -r '[.subfolder, .name] | map(select(. != "" and . != null)) | join("/")' <<<"${uploaded}")
      jq --arg image "${image}" \
        '.prompt |= map_values(if .class_type == "LoadImage" then .inputs.image = $image else . end)' \
        "${payload}" > "${payload}.tmp"
      mv "${payload}.tmp" "${payload}"
    fi

    queued=$(curl -sS --fail-with-body --max-time 30 -H 'content-type: application/json' \
      -d "@${payload}" "${COMFY_URL}/prompt")
    echo "queued: ${queued}"
    id=$(jq -r '.prompt_id // empty' <<<"${queued}")
    if [ -z "${id}" ]; then
      echo "submit: ComfyUI's reply has no prompt_id" >&2
      exit 1
    fi
    echo "${id}" > "${DECREE_RUN_DIR}/comfy-prompt-id"
    echo "prompt ${id}"
  video_i2v_wan2.2_14B_long.json: |
    {
      "client_id": "70b193468f844b99ada3c47dd94e287e",
      "prompt": {
        "84": {
          "inputs": {
            "clip_name": "umt5_xxl_fp8_e4m3fn_scaled.safetensors",
            "type": "wan",
            "device": "default"
          },
          "class_type": "CLIPLoader",
          "_meta": { "title": "Load CLIP" }
        },
        "85": {
          "inputs": {
            "add_noise": "disable",
            "noise_seed": 0,
            "steps": 4,
            "cfg": 1,
            "sampler_name": "euler",
            "scheduler": "simple",
            "start_at_step": 2,
            "end_at_step": 4,
            "return_with_leftover_noise": "disable",
            "model": ["103", 0],
            "positive": ["98", 0],
            "negative": ["98", 1],
            "latent_image": ["86", 0]
          },
          "class_type": "KSamplerAdvanced",
          "_meta": { "title": "KSampler (Advanced)" }
        },
        "86": {
          "inputs": {
            "add_noise": "enable",
            "noise_seed": 633590501442481,
            "steps": 4,
            "cfg": 1,
            "sampler_name": "euler",
            "scheduler": "simple",
            "start_at_step": 0,
            "end_at_step": 2,
            "return_with_leftover_noise": "enable",
            "model": ["104", 0],
            "positive": ["98", 0],
            "negative": ["98", 1],
            "latent_image": ["98", 2]
          },
          "class_type": "KSamplerAdvanced",
          "_meta": { "title": "KSampler (Advanced)" }
        },
        "87": {
          "inputs": { "samples": ["85", 0], "vae": ["90", 0] },
          "class_type": "VAEDecode",
          "_meta": { "title": "VAE Decode" }
        },
        "89": {
          "inputs": {
            "text": "色调艳丽，过曝，静态，细节模糊不清，字幕，风格，作品，画作，画面，静止，整体发灰，最差质量，低质量，JPEG压缩残留，丑陋的，残缺的，多余的手指，画得不好的手部，画得不好的脸部，畸形的，毁容的，形态畸形的肢体，手指融合，静止不动的画面，杂乱的背景，三条腿，背景人很多，倒着走",
            "clip": ["84", 0]
          },
          "class_type": "CLIPTextEncode",
          "_meta": { "title": "CLIP Text Encode (Negative Prompt)" }
        },
        "90": {
          "inputs": { "vae_name": "wan_2.1_vae.safetensors" },
          "class_type": "VAELoader",
          "_meta": { "title": "Load VAE" }
        },
        "93": {
          "inputs": {
            "text": "ancient warrior kneels down at the solid gold plates he is carrying",
            "clip": ["84", 0]
          },
          "class_type": "CLIPTextEncode",
          "_meta": { "title": "CLIP Text Encode (Positive Prompt)" }
        },
        "94": {
          "inputs": { "fps": 16, "images": ["87", 0] },
          "class_type": "CreateVideo",
          "_meta": { "title": "Create Video" }
        },
        "95": {
          "inputs": {
            "unet_name": "wan2.2_i2v_high_noise_14B_fp8_scaled.safetensors",
            "weight_dtype": "default"
          },
          "class_type": "UNETLoader",
          "_meta": { "title": "Load Diffusion Model" }
        },
        "96": {
          "inputs": {
            "unet_name": "wan2.2_i2v_low_noise_14B_fp8_scaled.safetensors",
            "weight_dtype": "default"
          },
          "class_type": "UNETLoader",
          "_meta": { "title": "Load Diffusion Model" }
        },
        "97": {
          "inputs": { "image": "moroni_00001_.png" },
          "class_type": "LoadImage",
          "_meta": { "title": "Load Image" }
        },
        "98": {
          "inputs": {
            "width": 480,
            "height": 832,
            "length": 133,
            "batch_size": 1,
            "positive": ["93", 0],
            "negative": ["89", 0],
            "vae": ["90", 0],
            "start_image": ["97", 0]
          },
          "class_type": "WanImageToVideo",
          "_meta": { "title": "WanImageToVideo" }
        },
        "101": {
          "inputs": {
            "lora_name": "wan2.2_i2v_lightx2v_4steps_lora_v1_high_noise.safetensors",
            "strength_model": 1.0000000000000002,
            "model": ["95", 0]
          },
          "class_type": "LoraLoaderModelOnly",
          "_meta": { "title": "LoraLoaderModelOnly" }
        },
        "102": {
          "inputs": {
            "lora_name": "wan2.2_i2v_lightx2v_4steps_lora_v1_low_noise.safetensors",
            "strength_model": 1.0000000000000002,
            "model": ["96", 0]
          },
          "class_type": "LoraLoaderModelOnly",
          "_meta": { "title": "LoraLoaderModelOnly" }
        },
        "103": {
          "inputs": { "shift": 5.000000000000001, "model": ["102", 0] },
          "class_type": "ModelSamplingSD3",
          "_meta": { "title": "ModelSamplingSD3" }
        },
        "104": {
          "inputs": { "shift": 5.000000000000001, "model": ["101", 0] },
          "class_type": "ModelSamplingSD3",
          "_meta": { "title": "ModelSamplingSD3" }
        },
        "108": {
          "inputs": {
            "filename_prefix": "video/2026.02.22_wan",
            "format": "auto",
            "codec": "auto",
            "video": ["94", 0]
          },
          "class_type": "SaveVideo",
          "_meta": { "title": "Save Video" }
        }
      },
      "extra_data": {
        "extra_pnginfo": {
          "workflow": {
            "id": "ec7da562-7e21-4dac-a0d2-f4441e1efd3b",
            "revision": 0,
            "last_node_id": 116,
            "last_link_id": 214,
            "nodes": [
              {
                "id": 38,
                "type": "CLIPLoader",
                "pos": [85.98029663932107, 1633.976360209474],
                "size": [415.6666564941406, 152],
                "flags": {},
                "order": 0,
                "mode": 4,
                "inputs": [],
                "outputs": [
                  {
                    "name": "CLIP",
                    "type": "CLIP",
                    "slot_index": 0,
                    "links": [74, 75]
                  }
                ],
                "properties": {
                  "cnr_id": "comfy-core",
                  "ver": "0.3.45",
                  "Node name for S&R": "CLIPLoader",
                  "models": [
                    {
                      "name": "umt5_xxl_fp8_e4m3fn_scaled.safetensors",
                      "url": "https://huggingface.co/Comfy-Org/Wan_2.1_ComfyUI_repackaged/resolve/main/split_files/text_encoders/umt5_xxl_fp8_e4m3fn_scaled.safetensors",
                      "directory": "text_encoders"
                    }
                  ],
                  "ue_properties": {
                    "widget_ue_connectable": {},
                    "version": "7.1",
                    "input_ue_unconnectable": {}
                  }
                },
                "widgets_values": [
                  "umt5_xxl_fp8_e4m3fn_scaled.safetensors",
                  "wan",
                  "default"
                ]
              },
              {
                "id": 7,
                "type": "CLIPTextEncode",
                "pos": [541.9795640107844, 1898.0109215211137],
                "size": [510.3333435058594, 121.5],
                "flags": {},
                "order": 16,
                "mode": 4,
                "inputs": [{ "name": "clip", "type": "CLIP", "link": 75 }],
                "outputs": [
                  {
                    "name": "CONDITIONING",
                    "type": "CONDITIONING",
                    "slot_index": 0,
                    "links": [135]
                  }
                ],
                "title": "CLIP Text Encode (Negative Prompt)",
                "properties": {
                  "cnr_id": "comfy-core",
                  "ver": "0.3.45",
                  "Node name for S&R": "CLIPTextEncode",
                  "ue_properties": {
                    "widget_ue_connectable": {},
                    "version": "7.1",
                    "input_ue_unconnectable": {}
                  }
                },
                "widgets_values": [
                  "色调艳丽，过曝，静态，细节模糊不清，字幕，风格，作品，画作，画面，静止，整体发灰，最差质量，低质量，JPEG压缩残留，丑陋的，残缺的，多余的手指，画得不好的手部，画得不好的脸部，畸形的，毁容的，形态畸形的肢体，手指融合，静止不动的画面，杂乱的背景，三条腿，背景人很多，倒着走"
                ],
                "color": "#322",
                "bgcolor": "#533"
              },
              {
                "id": 39,
                "type": "VAELoader",
                "pos": [85.98029663932107, 1825.9726864482163],
                "size": [413.6833190917969, 82.66666412353516],
                "flags": {},
                "order": 1,
                "mode": 4,
                "inputs": [],
                "outputs": [
                  {
                    "name": "VAE",
                    "type": "VAE",
                    "slot_index": 0,
                    "links": [76, 141]
                  }
                ],
                "properties": {
                  "cnr_id": "comfy-core",
                  "ver": "0.3.45",
                  "Node name for S&R": "VAELoader",
                  "models": [
                    {
                      "name": "wan_2.1_vae.safetensors",
                      "url": "https://huggingface.co/Comfy-Org/Wan_2.2_ComfyUI_Repackaged/resolve/main/split_files/vae/wan_2.1_vae.safetensors",
                      "directory": "vae"
                    }
                  ],
                  "ue_properties": {
                    "widget_ue_connectable": {},
                    "version": "7.1",
                    "input_ue_unconnectable": {}
                  }
                },
                "widgets_values": ["wan_2.1_vae.safetensors"]
              },
              {
                "id": 54,
                "type": "ModelSamplingSD3",
                "pos": [806.0142420020509, 1321.9903226259673],
                "size": [252, 80],
                "flags": {},
                "order": 17,
                "mode": 4,
                "inputs": [{ "name": "model", "type": "MODEL", "link": 110 }],
                "outputs": [
                  {
                    "name": "MODEL",
                    "type": "MODEL",
                    "slot_index": 0,
                    "links": [147]
                  }
                ],
                "properties": {
                  "cnr_id": "comfy-core",
                  "ver": "0.3.45",
                  "Node name for S&R": "ModelSamplingSD3",
                  "ue_properties": {
                    "widget_ue_connectable": {},
                    "version": "7.1",
                    "input_ue_unconnectable": {}
                  }
                },
                "widgets_values": [8.000000000000002]
              },
              {
                "id": 55,
                "type": "ModelSamplingSD3",
                "pos": [806.0142420020509, 1477.9833997575342],
                "size": [252, 80],
                "flags": {},
                "order": 18,
                "mode": 4,
                "inputs": [{ "name": "model", "type": "MODEL", "link": 112 }],
                "outputs": [
                  {
                    "name": "MODEL",
                    "type": "MODEL",
                    "slot_index": 0,
                    "links": [148]
                  }
                ],
                "properties": {
                  "cnr_id": "comfy-core",
                  "ver": "0.3.45",
                  "Node name for S&R": "ModelSamplingSD3",
                  "ue_properties": {
                    "widget_ue_connectable": {},
                    "version": "7.1",
                    "input_ue_unconnectable": {}
                  }
                },
                "widgets_values": [8]
              },
              {
                "id": 6,
                "type": "CLIPTextEncode",
                "pos": [541.9795640107844, 1657.9997619730398],
                "size": [507.4166564941406, 121.5],
                "flags": {},
                "order": 15,
                "mode": 4,
                "inputs": [{ "name": "clip", "type": "CLIP", "link": 74 }],
                "outputs": [
                  {
                    "name": "CONDITIONING",
                    "type": "CONDITIONING",
                    "slot_index": 0,
                    "links": [134]
                  }
                ],
                "title": "CLIP Text Encode (Positive Prompt)",
                "properties": {
                  "cnr_id": "comfy-core",
                  "ver": "0.3.45",
                  "Node name for S&R": "CLIPTextEncode",
                  "ue_properties": {
                    "widget_ue_connectable": {},
                    "version": "7.1",
                    "input_ue_unconnectable": {}
                  }
                },
                "widgets_values": [
                  "The white dragon warrior stands still, eyes full of determination and strength. The camera slowly moves closer or circles around the warrior, highlighting the powerful presence and heroic spirit of the character."
                ],
                "color": "#232",
                "bgcolor": "#353"
              },
              {
                "id": 37,
                "type": "UNETLoader",
                "pos": [85.98029663932107, 1321.9903226259673],
                "size": [416.1000061035156, 117.33332824707031],
                "flags": {},
                "order": 2,
                "mode": 4,
                "inputs": [],
                "outputs": [
                  {
                    "name": "MODEL",
                    "type": "MODEL",
                    "slot_index": 0,
                    "links": [110]
                  }
                ],
                "properties": {
                  "cnr_id": "comfy-core",
                  "ver": "0.3.45",
                  "Node name for S&R": "UNETLoader",
                  "models": [
                    {
                      "name": "wan2.2_i2v_high_noise_14B_fp8_scaled.safetensors",
                      "url": "https://huggingface.co/Comfy-Org/Wan_2.2_ComfyUI_Repackaged/resolve/main/split_files/diffusion_models/wan2.2_i2v_high_noise_14B_fp8_scaled.safetensors",
                      "directory": "diffusion_models"
                    }
                  ],
                  "ue_properties": {
                    "widget_ue_connectable": {},
                    "version": "7.1",
                    "input_ue_unconnectable": {}
                  }
                },
                "widgets_values": [
                  "wan2.2_i2v_high_noise_14B_fp8_scaled.safetensors",
                  "default"
                ]
              },
              {
                "id": 56,
                "type": "UNETLoader",
                "pos": [85.98029663932107, 1477.9833997575342],
                "size": [416.1000061035156, 117.33332824707031],
                "flags": {},
                "order": 3,
                "mode": 4,
                "inputs": [],
                "outputs": [
                  {
                    "name": "MODEL",
                    "type": "MODEL",
                    "slot_index": 0,
                    "links": [112]
                  }
                ],
                "properties": {
                  "cnr_id": "comfy-core",
                  "ver": "0.3.45",
                  "Node name for S&R": "UNETLoader",
                  "models": [
                    {
                      "name": "wan2.2_i2v_low_noise_14B_fp8_scaled.safetensors",
                      "url": "https://huggingface.co/Comfy-Org/Wan_2.2_ComfyUI_Repackaged/resolve/main/split_files/diffusion_models/wan2.2_i2v_low_noise_14B_fp8_scaled.safetensors",
                      "directory": "diffusion_models"
                    }
                  ],
                  "ue_properties": {
                    "widget_ue_connectable": {},
                    "version": "7.1",
                    "input_ue_unconnectable": {}
                  }
                },
                "widgets_values": [
                  "wan2.2_i2v_low_noise_14B_fp8_scaled.safetensors",
                  "default"
                ]
              },
              {
                "id": 63,
                "type": "WanImageToVideo",
                "pos": [577.9828131179602, 2233.988507330013],
                "size": [411.1166687011719, 272],
                "flags": {},
                "order": 23,
                "mode": 4,
                "inputs": [
                  { "name": "positive", "type": "CONDITIONING", "link": 134 },
                  { "name": "negative", "type": "CONDITIONING", "link": 135 },
                  { "name": "vae", "type": "VAE", "link": 141 },
                  {
                    "name": "clip_vision_output",
                    "shape": 7,
                    "type": "CLIP_VISION_OUTPUT",
                    "link": null
                  },
                  {
                    "name": "start_image",
                    "shape": 7,
                    "type": "IMAGE",
                    "link": 133
                  }
                ],
                "outputs": [
                  {
                    "name": "positive",
                    "type": "CONDITIONING",
                    "slot_index": 0,
                    "links": [136, 138]
                  },
                  {
                    "name": "negative",
                    "type": "CONDITIONING",
                    "slot_index": 1,
                    "links": [137, 139]
                  },
                  {
                    "name": "latent",
                    "type": "LATENT",
                    "slot_index": 2,
                    "links": [140]
                  }
                ],
                "properties": {
                  "cnr_id": "comfy-core",
                  "ver": "0.3.45",
                  "Node name for S&R": "WanImageToVideo",
                  "ue_properties": {
                    "widget_ue_connectable": {},
                    "version": "7.1",
                    "input_ue_unconnectable": {}
                  }
                },
                "widgets_values": [640, 640, 81, 1]
              },
              {
                "id": 84,
                "type": "CLIPLoader",
                "pos": [74.00044929571095, 38.01106964659243],
                "size": [415.6666564941406, 152],
                "flags": {},
                "order": 4,
                "mode": 0,
                "inputs": [],
                "outputs": [
                  {
                    "name": "CLIP",
                    "type": "CLIP",
                    "slot_index": 0,
                    "links": [178, 181]
                  }
                ],
                "properties": {
                  "cnr_id": "comfy-core",
                  "ver": "0.3.45",
                  "Node name for S&R": "CLIPLoader",
                  "models": [
                    {
                      "name": "umt5_xxl_fp8_e4m3fn_scaled.safetensors",
                      "url": "https://huggingface.co/Comfy-Org/Wan_2.1_ComfyUI_repackaged/resolve/main/split_files/text_encoders/umt5_xxl_fp8_e4m3fn_scaled.safetensors",
                      "directory": "text_encoders"
                    }
                  ],
                  "ue_properties": {
                    "widget_ue_connectable": {},
                    "version": "7.1",
                    "input_ue_unconnectable": {}
                  }
                },
                "widgets_values": [
                  "umt5_xxl_fp8_e4m3fn_scaled.safetensors",
                  "wan",
                  "default"
                ]
              },
              {
                "id": 90,
                "type": "VAELoader",
                "pos": [74.00044929571095, 229.97554234716222],
                "size": [413.6833190917969, 82.66666412353516],
                "flags": {},
                "order": 5,
                "mode": 0,
                "inputs": [],
                "outputs": [
                  {
                    "name": "VAE",
                    "type": "VAE",
                    "slot_index": 0,
                    "links": [176, 185]
                  }
                ],
                "properties": {
                  "cnr_id": "comfy-core",
                  "ver": "0.3.45",
                  "Node name for S&R": "VAELoader",
                  "models": [
                    {
                      "name": "wan_2.1_vae.safetensors",
                      "url": "https://huggingface.co/Comfy-Org/Wan_2.2_ComfyUI_Repackaged/resolve/main/split_files/vae/wan_2.1_vae.safetensors",
                      "directory": "vae"
                    }
                  ],
                  "ue_properties": {
                    "widget_ue_connectable": {},
                    "version": "7.1",
                    "input_ue_unconnectable": {}
                  }
                },
                "widgets_values": ["wan_2.1_vae.safetensors"]
              },
              {
                "id": 95,
                "type": "UNETLoader",
                "pos": [62.02060195210083, -274.00679230518017],
                "size": [416.1000061035156, 117.33332824707031],
                "flags": {},
                "order": 6,
                "mode": 0,
                "inputs": [],
                "outputs": [
                  {
                    "name": "MODEL",
                    "type": "MODEL",
                    "slot_index": 0,
                    "links": [194]
                  }
                ],
                "properties": {
                  "cnr_id": "comfy-core",
                  "ver": "0.3.45",
                  "Node name for S&R": "UNETLoader",
                  "models": [
                    {
                      "name": "wan2.2_i2v_high_noise_14B_fp8_scaled.safetensors",
                      "url": "https://huggingface.co/Comfy-Org/Wan_2.2_ComfyUI_Repackaged/resolve/main/split_files/diffusion_models/wan2.2_i2v_high_noise_14B_fp8_scaled.safetensors",
                      "directory": "diffusion_models"
                    }
                  ],
                  "ue_properties": {
                    "widget_ue_connectable": {},
                    "version": "7.1",
                    "input_ue_unconnectable": {}
                  }
                },
                "widgets_values": [
                  "wan2.2_i2v_high_noise_14B_fp8_scaled.safetensors",
                  "default"
                ]
              },
              {
                "id": 96,
                "type": "UNETLoader",
                "pos": [62.02060195210083, -118.01377351342683],
                "size": [416.1000061035156, 117.33332824707031],
                "flags": {},
                "order": 7,
                "mode": 0,
                "inputs": [],
                "outputs": [
                  {
                    "name": "MODEL",
                    "type": "MODEL",
                    "slot_index": 0,
                    "links": [196]
                  }
                ],
                "properties": {
                  "cnr_id": "comfy-core",
                  "ver": "0.3.45",
                  "Node name for S&R": "UNETLoader",
                  "models": [
                    {
                      "name": "wan2.2_i2v_low_noise_14B_fp8_scaled.safetensors",
                      "url": "https://huggingface.co/Comfy-Org/Wan_2.2_ComfyUI_Repackaged/resolve/main/split_files/diffusion_models/wan2.2_i2v_low_noise_14B_fp8_scaled.safetensors",
                      "directory": "diffusion_models"
                    }
                  ],
                  "ue_properties": {
                    "widget_ue_connectable": {},
                    "version": "7.1",
                    "input_ue_unconnectable": {}
                  }
                },
                "widgets_values": [
                  "wan2.2_i2v_low_noise_14B_fp8_scaled.safetensors",
                  "default"
                ]
              },
              {
                "id": 103,
                "type": "ModelSamplingSD3",
                "pos": [890.0007042396394, -118.01377351342683],
                "size": [252, 80],
                "flags": { "collapsed": false },
                "order": 26,
                "mode": 0,
                "inputs": [{ "name": "model", "type": "MODEL", "link": 189 }],
                "outputs": [
                  {
                    "name": "MODEL",
                    "type": "MODEL",
                    "slot_index": 0,
                    "links": [192]
                  }
                ],
                "properties": {
                  "cnr_id": "comfy-core",
                  "ver": "0.3.45",
                  "Node name for S&R": "ModelSamplingSD3",
                  "ue_properties": {
                    "widget_ue_connectable": {},
                    "version": "7.1",
                    "input_ue_unconnectable": {}
                  }
                },
                "widgets_values": [5.000000000000001]
              },
              {
                "id": 101,
                "type": "LoraLoaderModelOnly",
                "pos": [541.9795640107844, -274.00679230518017],
                "size": [336, 114.66667175292969],
                "flags": {},
                "order": 21,
                "mode": 0,
                "inputs": [{ "name": "model", "type": "MODEL", "link": 194 }],
                "outputs": [{ "name": "MODEL", "type": "MODEL", "links": [190] }],
                "properties": {
                  "cnr_id": "comfy-core",
                  "ver": "0.3.49",
                  "Node name for S&R": "LoraLoaderModelOnly",
                  "models": [
                    {
                      "name": "wan2.2_i2v_lightx2v_4steps_lora_v1_high_noise.safetensors",
                      "url": "https://huggingface.co/Comfy-Org/Wan_2.2_ComfyUI_Repackaged/resolve/main/split_files/loras/wan2.2_i2v_lightx2v_4steps_lora_v1_high_noise.safetensors",
                      "directory": "loras"
                    }
                  ],
                  "ue_properties": {
                    "widget_ue_connectable": {},
                    "version": "7.1",
                    "input_ue_unconnectable": {}
                  }
                },
                "widgets_values": [
                  "wan2.2_i2v_lightx2v_4steps_lora_v1_high_noise.safetensors",
                  1.0000000000000002
                ]
              },
              {
                "id": 102,
                "type": "LoraLoaderModelOnly",
                "pos": [541.9795640107844, -118.01377351342683],
                "size": [336, 114.66667175292969],
                "flags": {},
                "order": 22,
                "mode": 0,
                "inputs": [{ "name": "model", "type": "MODEL", "link": 196 }],
                "outputs": [{ "name": "MODEL", "type": "MODEL", "links": [189] }],
                "properties": {
                  "cnr_id": "comfy-core",
                  "ver": "0.3.49",
                  "Node name for S&R": "LoraLoaderModelOnly",
                  "models": [
                    {
                      "name": "wan2.2_i2v_lightx2v_4steps_lora_v1_low_noise.safetensors",
                      "url": "https://huggingface.co/Comfy-Org/Wan_2.2_ComfyUI_Repackaged/resolve/main/split_files/loras/wan2.2_i2v_lightx2v_4steps_lora_v1_low_noise.safetensors",
                      "directory": "loras"
                    }
                  ],
                  "ue_properties": {
                    "widget_ue_connectable": {},
                    "version": "7.1",
                    "input_ue_unconnectable": {}
                  }
                },
                "widgets_values": [
                  "wan2.2_i2v_lightx2v_4steps_lora_v1_low_noise.safetensors",
                  1.0000000000000002
                ]
              },
              {
                "id": 62,
                "type": "LoadImage",
                "pos": [97.99205586091739, 2090.007247759856],
                "size": [378, 344],
                "flags": {},
                "order": 8,
                "mode": 4,
                "inputs": [],
                "outputs": [
                  {
                    "name": "IMAGE",
                    "type": "IMAGE",
                    "slot_index": 0,
                    "links": [133]
                  },
                  { "name": "MASK", "type": "MASK", "slot_index": 1, "links": null }
                ],
                "properties": {
                  "cnr_id": "comfy-core",
                  "ver": "0.3.45",
                  "Node name for S&R": "LoadImage",
                  "ue_properties": {
                    "widget_ue_connectable": {},
                    "version": "7.1",
                    "input_ue_unconnectable": {}
                  }
                },
                "widgets_values": ["video_wan2_2_14B_i2v_input_image.jpg", "image"]
              },
              {
                "id": 106,
                "type": "MarkdownNote",
                "pos": [-418.0019796732078, 1214.012312162986],
                "size": [444, 156],
                "flags": {},
                "order": 9,
                "mode": 0,
                "inputs": [],
                "outputs": [],
                "properties": {
                  "ue_properties": {
                    "widget_ue_connectable": {},
                    "version": "7.1",
                    "input_ue_unconnectable": {}
                  }
                },
                "widgets_values": [
                  "1. Box-select then use Ctrl + B to enable\n2. If you don't want to run both groups simultaneously, don't forget to use **Ctrl + B** to disable the **fp8_scaled + 4steps LoRA** group after enabling the **fp8_scaled** group, or try the [partial - execution](https://docs.comfy.org/interface/features/partial-execution) feature."
                ],
                "color": "#432",
                "bgcolor": "#653"
              },
              {
                "id": 67,
                "type": "Note",
                "pos": [614.0179157633086, 986.0127368170679],
                "size": [468, 97.5],
                "flags": {},
                "order": 10,
                "mode": 0,
                "inputs": [],
                "outputs": [],
                "title": "Video Size",
                "properties": {
                  "ue_properties": {
                    "widget_ue_connectable": {},
                    "version": "7.1",
                    "input_ue_unconnectable": {}
                  }
                },
                "widgets_values": [
                  "By default, we set the video to a smaller size for users with low VRAM. If you have enough VRAM, you can change the size"
                ],
                "color": "#432",
                "bgcolor": "#653"
              },
              {
                "id": 104,
                "type": "ModelSamplingSD3",
                "pos": [890.0007042396394, -274.00679230518017],
                "size": [252, 80],
                "flags": {},
                "order": 25,
                "mode": 0,
                "inputs": [{ "name": "model", "type": "MODEL", "link": 190 }],
                "outputs": [
                  {
                    "name": "MODEL",
                    "type": "MODEL",
                    "slot_index": 0,
                    "links": [195]
                  }
                ],
                "properties": {
                  "cnr_id": "comfy-core",
                  "ver": "0.3.45",
                  "Node name for S&R": "ModelSamplingSD3",
                  "ue_properties": {
                    "widget_ue_connectable": {},
                    "version": "7.1",
                    "input_ue_unconnectable": {}
                  }
                },
                "widgets_values": [5.000000000000001]
              },
              {
                "id": 87,
                "type": "VAEDecode",
                "pos": [1273.9613864993248, 577.9966825760173],
                "size": [252, 72],
                "flags": {},
                "order": 32,
                "mode": 0,
                "inputs": [
                  { "name": "samples", "type": "LATENT", "link": 175 },
                  { "name": "vae", "type": "VAE", "link": 176 }
                ],
                "outputs": [
                  {
                    "name": "IMAGE",
                    "type": "IMAGE",
                    "slot_index": 0,
                    "links": [182]
                  }
                ],
                "properties": {
                  "cnr_id": "comfy-core",
                  "ver": "0.3.45",
                  "Node name for S&R": "VAEDecode",
                  "ue_properties": {
                    "widget_ue_connectable": {},
                    "version": "7.1",
                    "input_ue_unconnectable": {}
                  }
                },
                "widgets_values": []
              },
              {
                "id": 115,
                "type": "Note",
                "pos": [37.99720018853526, -562.0012233234804],
                "size": [432, 97.5],
                "flags": {},
                "order": 11,
                "mode": 0,
                "inputs": [],
                "outputs": [],
                "title": "About 4 Steps LoRA",
                "properties": {
                  "ue_properties": {
                    "widget_ue_connectable": {},
                    "version": "7.1",
                    "input_ue_unconnectable": {}
                  }
                },
                "widgets_values": [
                  "Using the Wan2.2 Lighting LoRA will result in the loss of video dynamics, but it will reduce the generation time. This template provides two workflows, and you can enable one as needed."
                ],
                "color": "#432",
                "bgcolor": "#653"
              },
              {
                "id": 86,
                "type": "KSamplerAdvanced",
                "pos": [1189.9749242617363, -297.99834053057305],
                "size": [365.70001220703125, 505.20001220703125],
                "flags": {},
                "order": 28,
                "mode": 0,
                "inputs": [
                  { "name": "model", "type": "MODEL", "link": 195 },
                  { "name": "positive", "type": "CONDITIONING", "link": 172 },
                  { "name": "negative", "type": "CONDITIONING", "link": 173 },
                  { "name": "latent_image", "type": "LATENT", "link": 174 }
                ],
                "outputs": [{ "name": "LATENT", "type": "LATENT", "links": [170] }],
                "properties": {
                  "cnr_id": "comfy-core",
                  "ver": "0.3.45",
                  "Node name for S&R": "KSamplerAdvanced",
                  "ue_properties": {
                    "widget_ue_connectable": {},
                    "version": "7.1",
                    "input_ue_unconnectable": {}
                  }
                },
                "widgets_values": [
                  "enable",
                  633590501442481,
                  "randomize",
                  4,
                  1,
                  "euler",
                  "simple",
                  0,
                  2,
                  "enable"
                ]
              },
              {
                "id": 85,
                "type": "KSamplerAdvanced",
                "pos": [1606.0837607523117, -297.99834053057305],
                "size": [365.70001220703125, 505.20001220703125],
                "flags": {},
                "order": 30,
                "mode": 0,
                "inputs": [
                  { "name": "model", "type": "MODEL", "link": 192 },
                  { "name": "positive", "type": "CONDITIONING", "link": 168 },
                  { "name": "negative", "type": "CONDITIONING", "link": 169 },
                  { "name": "latent_image", "type": "LATENT", "link": 170 }
                ],
                "outputs": [{ "name": "LATENT", "type": "LATENT", "links": [175] }],
                "properties": {
                  "cnr_id": "comfy-core",
                  "ver": "0.3.45",
                  "Node name for S&R": "KSamplerAdvanced",
                  "ue_properties": {
                    "widget_ue_connectable": {},
                    "version": "7.1",
                    "input_ue_unconnectable": {}
                  }
                },
                "widgets_values": [
                  "disable",
                  0,
                  "fixed",
                  4,
                  1,
                  "euler",
                  "simple",
                  2,
                  4,
                  "disable"
                ]
              },
              {
                "id": 58,
                "type": "KSamplerAdvanced",
                "pos": [1489.9811145016329, 1334.0020235077502],
                "size": [365.70001220703125, 606.9000244140625],
                "flags": {},
                "order": 29,
                "mode": 4,
                "inputs": [
                  { "name": "model", "type": "MODEL", "link": 148 },
                  { "name": "positive", "type": "CONDITIONING", "link": 138 },
                  { "name": "negative", "type": "CONDITIONING", "link": 139 },
                  { "name": "latent_image", "type": "LATENT", "link": 113 }
                ],
                "outputs": [{ "name": "LATENT", "type": "LATENT", "links": [124] }],
                "properties": {
                  "cnr_id": "comfy-core",
                  "ver": "0.3.45",
                  "Node name for S&R": "KSamplerAdvanced",
                  "ue_properties": {
                    "widget_ue_connectable": {},
                    "version": "7.1",
                    "input_ue_unconnectable": {}
                  }
                },
                "widgets_values": [
                  "disable",
                  0,
                  "fixed",
                  20,
                  3.5,
                  "euler",
                  "simple",
                  10,
                  10000,
                  "disable"
                ]
              },
              {
                "id": 109,
                "type": "CreateVideo",
                "pos": [1502.0246689215883, 2090.007247759856],
                "size": [324, 104],
                "flags": {},
                "order": 33,
                "mode": 4,
                "inputs": [
                  { "name": "images", "type": "IMAGE", "link": 198 },
                  { "name": "audio", "shape": 7, "type": "AUDIO", "link": null }
                ],
                "outputs": [{ "name": "VIDEO", "type": "VIDEO", "links": [199] }],
                "properties": {
                  "cnr_id": "comfy-core",
                  "ver": "0.3.49",
                  "Node name for S&R": "CreateVideo",
                  "ue_properties": {
                    "widget_ue_connectable": {},
                    "version": "7.1",
                    "input_ue_unconnectable": {}
                  }
                },
                "widgets_values": [16]
              },
              {
                "id": 8,
                "type": "VAEDecode",
                "pos": [1106.0204322419474, 2101.9872117830932],
                "size": [252, 72],
                "flags": {},
                "order": 31,
                "mode": 4,
                "inputs": [
                  { "name": "samples", "type": "LATENT", "link": 124 },
                  { "name": "vae", "type": "VAE", "link": 76 }
                ],
                "outputs": [
                  {
                    "name": "IMAGE",
                    "type": "IMAGE",
                    "slot_index": 0,
                    "links": [198]
                  }
                ],
                "properties": {
                  "cnr_id": "comfy-core",
                  "ver": "0.3.45",
                  "Node name for S&R": "VAEDecode",
                  "ue_properties": {
                    "widget_ue_connectable": {},
                    "version": "7.1",
                    "input_ue_unconnectable": {}
                  }
                },
                "widgets_values": []
              },
              {
                "id": 57,
                "type": "KSamplerAdvanced",
                "pos": [1106.0204322419474, 1334.0020235077502],
                "size": [372, 450],
                "flags": {},
                "order": 27,
                "mode": 4,
                "inputs": [
                  { "name": "model", "type": "MODEL", "link": 147 },
                  { "name": "positive", "type": "CONDITIONING", "link": 136 },
                  { "name": "negative", "type": "CONDITIONING", "link": 137 },
                  { "name": "latent_image", "type": "LATENT", "link": 140 }
                ],
                "outputs": [{ "name": "LATENT", "type": "LATENT", "links": [113] }],
                "properties": {
                  "cnr_id": "comfy-core",
                  "ver": "0.3.45",
                  "Node name for S&R": "KSamplerAdvanced",
                  "ue_properties": {
                    "widget_ue_connectable": {},
                    "version": "7.1",
                    "input_ue_unconnectable": {}
                  }
                },
                "widgets_values": [
                  "enable",
                  678723440296424,
                  "randomize",
                  20,
                  3.5,
                  "euler",
                  "simple",
                  0,
                  10,
                  "enable"
                ]
              },
              {
                "id": 61,
                "type": "SaveVideo",
                "pos": [1897.9970520630563, 1334.0020235077502],
                "size": [1188, 1038],
                "flags": {},
                "order": 35,
                "mode": 4,
                "inputs": [{ "name": "video", "type": "VIDEO", "link": 199 }],
                "outputs": [],
                "properties": {
                  "cnr_id": "comfy-core",
                  "ver": "0.3.45",
                  "Node name for S&R": "SaveVideo",
                  "ue_properties": {
                    "widget_ue_connectable": {},
                    "version": "7.1",
                    "input_ue_unconnectable": {}
                  }
                },
                "widgets_values": ["video/ComfyUI", "auto", "auto"]
              },
              {
                "id": 66,
                "type": "MarkdownNote",
                "pos": [-562.0151219514443, -382.0166563063342],
                "size": [576, 716.5999755859375],
                "flags": {},
                "order": 12,
                "mode": 0,
                "inputs": [],
                "outputs": [],
                "title": "Model Links",
                "properties": {
                  "ue_properties": {
                    "widget_ue_connectable": {},
                    "version": "7.1",
                    "input_ue_unconnectable": {}
                  }
                },
                "widgets_values": [
                  "[Tutorial](https://docs.comfy.org/tutorials/video/wan/wan2_2\n)\n\n**Diffusion Model**\n- [wan2.2_i2v_high_noise_14B_fp8_scaled.safetensors](https://huggingface.co/Comfy-Org/Wan_2.2_ComfyUI_Repackaged/resolve/main/split_files/diffusion_models/wan2.2_i2v_high_noise_14B_fp8_scaled.safetensors)\n- [wan2.2_i2v_low_noise_14B_fp8_scaled.safetensors](https://huggingface.co/Comfy-Org/Wan_2.2_ComfyUI_Repackaged/resolve/main/split_files/diffusion_models/wan2.2_i2v_low_noise_14B_fp8_scaled.safetensors)\n\n**LoRA**\n- [wan2.2_i2v_lightx2v_4steps_lora_v1_low_noise.safetensors](https://huggingface.co/Comfy-Org/Wan_2.2_ComfyUI_Repackaged/resolve/main/split_files/loras/wan2.2_i2v_lightx2v_4steps_lora_v1_low_noise.safetensors)\n- [wan2.2_i2v_lightx2v_4steps_lora_v1_high_noise.safetensors](https://huggingface.co/Comfy-Org/Wan_2.2_ComfyUI_Repackaged/resolve/main/split_files/loras/wan2.2_i2v_lightx2v_4steps_lora_v1_high_noise.safetensors)\n\n**VAE**\n- [wan_2.1_vae.safetensors](https://huggingface.co/Comfy-Org/Wan_2.2_ComfyUI_Repackaged/resolve/main/split_files/vae/wan_2.1_vae.safetensors)\n\n**Text Encoder**   \n- [umt5_xxl_fp8_e4m3fn_scaled.safetensors](https://huggingface.co/Comfy-Org/Wan_2.1_ComfyUI_repackaged/resolve/main/split_files/text_encoders/umt5_xxl_fp8_e4m3fn_scaled.safetensors)\n\n\nFile save location\n\n```\nComfyUI/\n├───📂 models/\n│   ├───📂 diffusion_models/\n│   │   ├─── wan2.2_i2v_low_noise_14B_fp8_scaled.safetensors\n│   │   └─── wan2.2_i2v_high_noise_14B_fp8_scaled.safetensors\n│   ├───📂 loras/\n│   │   ├─── wan2.2_i2v_lightx2v_4steps_lora_v1_low_noise.safetensors\n│   │   └─── wan2.2_i2v_lightx2v_4steps_lora_v1_low_noise.safetensors\n│   ├───📂 text_encoders/\n│   │   └─── umt5_xxl_fp8_e4m3fn_scaled.safetensors \n│   └───📂 vae/\n│       └── wan_2.1_vae.safetensors\n```\n"
                ],
                "color": "#432",
                "bgcolor": "#653"
              },
              {
                "id": 94,
                "type": "CreateVideo",
                "pos": [1621.98252672818, 554.0051343506243],
                "size": [324, 104],
                "flags": {},
                "order": 34,
                "mode": 0,
                "inputs": [
                  { "name": "images", "type": "IMAGE", "link": 182 },
                  { "name": "audio", "shape": 7, "type": "AUDIO", "link": null }
                ],
                "outputs": [{ "name": "VIDEO", "type": "VIDEO", "links": [197] }],
                "properties": {
                  "cnr_id": "comfy-core",
                  "ver": "0.3.45",
                  "Node name for S&R": "CreateVideo",
                  "ue_properties": {
                    "widget_ue_connectable": {},
                    "version": "7.1",
                    "input_ue_unconnectable": {}
                  }
                },
                "widgets_values": [16]
              },
              {
                "id": 98,
                "type": "WanImageToVideo",
                "pos": [637.9776104505286, 637.9915965882128],
                "size": [411.1166687011719, 272],
                "flags": {},
                "order": 24,
                "mode": 0,
                "inputs": [
                  { "name": "positive", "type": "CONDITIONING", "link": 183 },
                  { "name": "negative", "type": "CONDITIONING", "link": 184 },
                  { "name": "vae", "type": "VAE", "link": 185 },
                  {
                    "name": "clip_vision_output",
                    "shape": 7,
                    "type": "CLIP_VISION_OUTPUT",
                    "link": null
                  },
                  {
                    "name": "start_image",
                    "shape": 7,
                    "type": "IMAGE",
                    "link": 186
                  }
                ],
                "outputs": [
                  {
                    "name": "positive",
                    "type": "CONDITIONING",
                    "slot_index": 0,
                    "links": [168, 172]
                  },
                  {
                    "name": "negative",
                    "type": "CONDITIONING",
                    "slot_index": 1,
                    "links": [169, 173]
                  },
                  {
                    "name": "latent",
                    "type": "LATENT",
                    "slot_index": 2,
                    "links": [174]
                  }
                ],
                "properties": {
                  "cnr_id": "comfy-core",
                  "ver": "0.3.45",
                  "Node name for S&R": "WanImageToVideo",
                  "ue_properties": {
                    "widget_ue_connectable": {},
                    "version": "7.1",
                    "input_ue_unconnectable": {}
                  }
                },
                "widgets_values": [480, 832, 133, 1]
              },
              {
                "id": 105,
                "type": "MarkdownNote",
                "pos": [-562.0151219514443, 337.98540634831625],
                "size": [576, 234.79998779296875],
                "flags": {},
                "order": 13,
                "mode": 0,
                "inputs": [],
                "outputs": [],
                "title": "VRAM Usage",
                "properties": {
                  "ue_properties": {
                    "version": "7.1",
                    "widget_ue_connectable": {},
                    "input_ue_unconnectable": {}
                  }
                },
                "widgets_values": [
                  "## GPU:RTX4090D 24GB\n\n| Model            | Size |VRAM Usage | 1st Generation | 2nd Generation |\n|---------------------|-------|-----------|---------------|-----------------|\n| fp8_scaled               |640*640| 84%               | ≈  536s              | ≈ 513s                   |\n| fp8_scaled +  4steps LoRA  | 640*640  | 83%                | ≈ 97s               | ≈ 71s                   |"
                ],
                "color": "#432",
                "bgcolor": "#653"
              },
              {
                "id": 108,
                "type": "SaveVideo",
                "pos": [2030.030317827776, -297.99834053057305],
                "size": [571.7000122070312, 995.75],
                "flags": {},
                "order": 36,
                "mode": 0,
                "inputs": [{ "name": "video", "type": "VIDEO", "link": 197 }],
                "outputs": [],
                "properties": {
                  "cnr_id": "comfy-core",
                  "ver": "0.3.49",
                  "Node name for S&R": "SaveVideo",
                  "ue_properties": {
                    "widget_ue_connectable": {},
                    "version": "7.1",
                    "input_ue_unconnectable": {}
                  }
                },
                "widgets_values": ["video/2026.02.22_wan", "auto", "auto"]
              },
              {
                "id": 97,
                "type": "LoadImage",
                "pos": [76.1989268277622, 411.52134111346516],
                "size": [378, 768],
                "flags": {},
                "order": 14,
                "mode": 0,
                "inputs": [],
                "outputs": [
                  {
                    "name": "IMAGE",
                    "type": "IMAGE",
                    "slot_index": 0,
                    "links": [186]
                  },
                  { "name": "MASK", "type": "MASK", "slot_index": 1, "links": null }
                ],
                "properties": {
                  "cnr_id": "comfy-core",
                  "ver": "0.3.45",
                  "Node name for S&R": "LoadImage",
                  "ue_properties": {
                    "widget_ue_connectable": {},
                    "version": "7.1",
                    "input_ue_unconnectable": {}
                  }
                },
                "widgets_values": ["moroni_00001_.png", "image"]
              },
              {
                "id": 93,
                "type": "CLIPTextEncode",
                "pos": [529.9995999875473, 110.01768454057083],
                "size": [612, 121.5],
                "flags": {},
                "order": 20,
                "mode": 0,
                "inputs": [{ "name": "clip", "type": "CLIP", "link": 181 }],
                "outputs": [
                  {
                    "name": "CONDITIONING",
                    "type": "CONDITIONING",
                    "slot_index": 0,
                    "links": [183]
                  }
                ],
                "title": "CLIP Text Encode (Positive Prompt)",
                "properties": {
                  "cnr_id": "comfy-core",
                  "ver": "0.3.45",
                  "Node name for S&R": "CLIPTextEncode",
                  "ue_properties": {
                    "widget_ue_connectable": {},
                    "version": "7.1",
                    "input_ue_unconnectable": {}
                  }
                },
                "widgets_values": [
                  "ancient warrior kneels down at the solid gold plates he is carrying"
                ],
                "color": "#232",
                "bgcolor": "#353"
              },
              {
                "id": 89,
                "type": "CLIPTextEncode",
                "pos": [529.9995999875473, 349.997107230099],
                "size": [612, 121.5],
                "flags": {},
                "order": 19,
                "mode": 0,
                "inputs": [{ "name": "clip", "type": "CLIP", "link": 178 }],
                "outputs": [
                  {
                    "name": "CONDITIONING",
                    "type": "CONDITIONING",
                    "slot_index": 0,
                    "links": [184]
                  }
                ],
                "title": "CLIP Text Encode (Negative Prompt)",
                "properties": {
                  "cnr_id": "comfy-core",
                  "ver": "0.3.45",
                  "Node name for S&R": "CLIPTextEncode",
                  "ue_properties": {
                    "widget_ue_connectable": {},
                    "version": "7.1",
                    "input_ue_unconnectable": {}
                  }
                },
                "widgets_values": [
                  "色调艳丽，过曝，静态，细节模糊不清，字幕，风格，作品，画作，画面，静止，整体发灰，最差质量，低质量，JPEG压缩残留，丑陋的，残缺的，多余的手指，画得不好的手部，画得不好的脸部，畸形的，毁容的，形态畸形的肢体，手指融合，静止不动的画面，杂乱的背景，三条腿，背景人很多，倒着走"
                ],
                "color": "#322",
                "bgcolor": "#533"
              }
            ],
            "links": [
              [74, 38, 0, 6, 0, "CLIP"],
              [75, 38, 0, 7, 0, "CLIP"],
              [76, 39, 0, 8, 1, "VAE"],
              [110, 37, 0, 54, 0, "MODEL"],
              [112, 56, 0, 55, 0, "MODEL"],
              [113, 57, 0, 58, 3, "LATENT"],
              [124, 58, 0, 8, 0, "LATENT"],
              [133, 62, 0, 63, 4, "IMAGE"],
              [134, 6, 0, 63, 0, "CONDITIONING"],
              [135, 7, 0, 63, 1, "CONDITIONING"],
              [136, 63, 0, 57, 1, "CONDITIONING"],
              [137, 63, 1, 57, 2, "CONDITIONING"],
              [138, 63, 0, 58, 1, "CONDITIONING"],
              [139, 63, 1, 58, 2, "CONDITIONING"],
              [140, 63, 2, 57, 3, "LATENT"],
              [141, 39, 0, 63, 2, "VAE"],
              [147, 54, 0, 57, 0, "MODEL"],
              [148, 55, 0, 58, 0, "MODEL"],
              [168, 98, 0, 85, 1, "CONDITIONING"],
              [169, 98, 1, 85, 2, "CONDITIONING"],
              [170, 86, 0, 85, 3, "LATENT"],
              [172, 98, 0, 86, 1, "CONDITIONING"],
              [173, 98, 1, 86, 2, "CONDITIONING"],
              [174, 98, 2, 86, 3, "LATENT"],
              [175, 85, 0, 87, 0, "LATENT"],
              [176, 90, 0, 87, 1, "VAE"],
              [178, 84, 0, 89, 0, "CLIP"],
              [181, 84, 0, 93, 0, "CLIP"],
              [182, 87, 0, 94, 0, "IMAGE"],
              [183, 93, 0, 98, 0, "CONDITIONING"],
              [184, 89, 0, 98, 1, "CONDITIONING"],
              [185, 90, 0, 98, 2, "VAE"],
              [186, 97, 0, 98, 4, "IMAGE"],
              [189, 102, 0, 103, 0, "MODEL"],
              [190, 101, 0, 104, 0, "MODEL"],
              [192, 103, 0, 85, 0, "MODEL"],
              [194, 95, 0, 101, 0, "MODEL"],
              [195, 104, 0, 86, 0, "MODEL"],
              [196, 96, 0, 102, 0, "MODEL"],
              [197, 94, 0, 108, 0, "VIDEO"],
              [198, 8, 0, 109, 0, "IMAGE"],
              [199, 109, 0, 61, 0, "VIDEO"]
            ],
            "groups": [
              {
                "id": 10,
                "title": "fp8_scaled",
                "bounding": [50, 1178, 3084, 1380],
                "color": "#3f789e",
                "font_size": 24,
                "flags": {}
              },
              {
                "id": 1,
                "title": "Step1 - Load models",
                "bounding": [62, 1226, 445.23724365234375, 685.676953125],
                "color": "#3f789e",
                "font_size": 24,
                "flags": {}
              },
              {
                "id": 2,
                "title": "Step2 - Upload start_image",
                "bounding": [62, 1946, 444, 564],
                "color": "#3f789e",
                "font_size": 24,
                "flags": {}
              },
              {
                "id": 3,
                "title": "Step4 -  Prompt",
                "bounding": [530, 1574, 534.3336181640625, 557.0473022460938],
                "color": "#3f789e",
                "font_size": 24,
                "flags": {}
              },
              {
                "id": 4,
                "title": "Step3 - Video size & length",
                "bounding": [530, 2150, 528, 360],
                "color": "#3f789e",
                "font_size": 24,
                "flags": {}
              },
              {
                "id": 15,
                "title": "fp8_scaled +  4steps LoRA",
                "bounding": [38, -418, 3096, 1344],
                "color": "#3f789e",
                "font_size": 24,
                "flags": {}
              },
              {
                "id": 11,
                "title": "Step1 - Load models",
                "bounding": [50, -370, 445.23724365234375, 685.676953125],
                "color": "#3f789e",
                "font_size": 24,
                "flags": {}
              },
              {
                "id": 12,
                "title": "Step2 - Upload start_image",
                "bounding": [50, 338, 444, 564],
                "color": "#3f789e",
                "font_size": 24,
                "flags": {}
              },
              {
                "id": 13,
                "title": "Step4 -  Prompt",
                "bounding": [518, 26, 636, 504],
                "color": "#3f789e",
                "font_size": 24,
                "flags": {}
              },
              {
                "id": 14,
                "title": "Step3 - Video size & length",
                "bounding": [518, 554, 636, 348],
                "color": "#3f789e",
                "font_size": 24,
                "flags": {}
              },
              {
                "id": 16,
                "title": "Lightx2v 4steps LoRA",
                "bounding": [518, -370, 636, 372],
                "color": "#3f789e",
                "font_size": 24,
                "flags": {}
              }
            ],
            "config": {},
            "extra": {
              "ds": {
                "scale": 0.5231003716419486,
                "offset": [454.70621080953015, 546.4714035402208]
              },
              "frontendVersion": "1.37.11",
              "workflowRendererVersion": "Vue",
              "VHS_latentpreview": false,
              "VHS_latentpreviewrate": 0,
              "VHS_MetadataImage": true,
              "VHS_KeepIntermediate": true,
              "ue_links": []
            },
            "version": 0.4
          }
        }
      }
    }
---
# comfy

Renders images and videos on [ComfyUI](https://github.com/comfyanonymous/ComfyUI), one message per render. The message names the workflow (`method`), its body is the prompt, and the render is saved in the repository at the path `output` gives. No model is involved: the scripts call ComfyUI's API with `curl` and `jq`.

ComfyUI's API is fire and forget: `POST /prompt` queues a workflow and answers at once, before anything renders. So a render is four steps: build, submit, await, fetch.

## What it does

1. `precheck` (on entry) fails fast if `curl` or `jq` is missing.
2. `build` patches `.decree/machines/comfy/scripts/<method>.json` with the message: the body is the prompt, and `width`, `height` and `seed` replace the workflow's own values when set. It fails, before anything is sent, on an unknown method (listing the methods) or a missing `method`, `output`, prompt or `input_image`.
3. `submit` uploads `input_image`, if any, queues the workflow and keeps the prompt id (`attempts: 3`).
4. `await` polls `GET /history/<prompt id>` every `COMFY_POLL_S` seconds (5) until the prompt finishes, within 3 hours.
5. `fetch` downloads each file the prompt saved to `output` plus its extension, or `output-<n>` plus it when there are several (`attempts: 3`).

## What it needs

- A running ComfyUI with the models its workflows load (FLUX2 for the images, WAN2.2 14B for the video), at `COMFY_URL` (`http://127.0.0.1:8188` by default, in `.decree/.env`).
- `curl` and `jq`.

## How to use it

A method is a workflow in `.decree/machines/comfy/scripts/`, by file stem, in ComfyUI's API format: `image_flux2_text_landscape` (an image from text), `image_flux2_text_image` (an image from text and a reference image) and `video_i2v_wan2.2_14B_long` (a video from a still image). A new method is a new workflow file there; keep it with `decree plugin export comfy`.

```yaml
---
machine: comfy
params:
  method: image_flux2_text_landscape
  output: output/unicorn_landscape   # required: repo path without extension
  width: 800                         # optional: 0 keeps the workflow's own value
  height: 400
  seed: 42                           # optional: -1 keeps the workflow's own value
---
A unicorn running along a rainbow into a pink sunset, fantasy art.
```

Put a message like this in `.decree/migrations/` and run `decree process`, or queue one and keep a daemon running:

```sh
echo "A lighthouse at dawn, oil painting" | decree emit --machine comfy --param method=image_flux2_text_landscape --param output=output/lighthouse
decree daemon
```

decree's repository has four demo messages and more on commissioning renders from another machine in `examples/text-to-media/`.
