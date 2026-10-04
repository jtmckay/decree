#!/usr/bin/env bash
# illustrated_post's write: ask Ollama for a short post from the message body,
# and write post.md to the run directory, linking the images without_comfy_wait collected.
set -euo pipefail
OLLAMA_URL="${OLLAMA_URL:-http://127.0.0.1:11434}"
OLLAMA_MODEL="${OLLAMA_MODEL:-gemma4:e4b}"
OLLAMA_TIMEOUT_S="${OLLAMA_TIMEOUT_S:-300}"

# The message body, after the frontmatter, is what the post is about.
request=$(awk 'NR == 1 && /^---$/ { fm = 1; next } fm == 1 && /^---$/ { fm = 2; next } fm != 1' "$DECREE_MESSAGE")

body=$(jq -n --arg model "$OLLAMA_MODEL" --arg request "$request" '{
  model: $model, stream: false,
  prompt: "Write a short post, at most 200 words of Markdown with no title, from this request:\n\n\($request)"
}')
reply=$(curl -fsS --max-time "$OLLAMA_TIMEOUT_S" -H 'content-type: application/json' -d "$body" "$OLLAMA_URL/api/generate")

post="$DECREE_RUN_DIR/post.md"
jq -r .response <<<"$reply" > "$post"
if [ -f "$DECREE_RUN_DIR/images.txt" ]; then
  while read -r image; do
    printf '\n![picture](%s)\n' "$image" >> "$post"
  done < "$DECREE_RUN_DIR/images.txt"
fi
cat "$post"
echo "wrote $post"
