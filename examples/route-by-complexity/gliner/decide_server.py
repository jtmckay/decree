"""GLiNER2.5-Decide over HTTP, for decree's gliner_router (docs/routers.md).

Loads fastino/GLiNER2.5-Decide-1B once, then serves POST /classify on 127.0.0.1:8090:

    in:  {"instructions": str, "labels": {name: description}, "text": str}
    out: {"event": <one of the labels>, "confidence": <the classifier's score for it>}

The output is a decree reply.json as it stands, so the router script writes it unchanged.

What the pages confirm (read 2026-10-04):
- The model card, https://huggingface.co/fastino/GLiNER2.5-Decide-1B: loading with
  AutoExtractor.from_pretrained, and classify_text(text, {task: {"labels": {label:
  description}}}) returning {task: label}. Apache-2.0; CPU or GPU.
- The gliner2 README, https://github.com/fastino-ai/GLiNER2: include_confidence=True
  returns {task: {"label": ..., "confidence": ...}}, and local inference needs the
  `gliner2[local]` extra.
Not confirmed: the pages show described labels and include_confidence in separate examples,
not together; they document no separate instructions argument (so the question goes in front
of the text), and no scores for every label (so the reply has no `probabilities`).
"""

import json
from http.server import BaseHTTPRequestHandler, HTTPServer

from gliner2 import AutoExtractor

MODEL = "fastino/GLiNER2.5-Decide-1B"
ADDRESS = ("127.0.0.1", 8090)

model = AutoExtractor.from_pretrained(MODEL)


class Classify(BaseHTTPRequestHandler):
    def do_POST(self):
        if self.path != "/classify":
            self.send_error(404)
            return
        req = json.loads(self.rfile.read(int(self.headers["content-length"])))
        result = model.classify_text(
            f"{req['instructions']}\n\n{req['text']}",
            {"decision": {"labels": req["labels"]}},
            include_confidence=True,
        )["decision"]
        body = json.dumps({"event": result["label"], "confidence": result["confidence"]})
        self.send_response(200)
        self.send_header("content-type", "application/json")
        self.end_headers()
        self.wfile.write(body.encode())


if __name__ == "__main__":
    print(f"{MODEL} loaded; serving POST http://{ADDRESS[0]}:{ADDRESS[1]}/classify", flush=True)
    HTTPServer(ADDRESS, Classify).serve_forever()
