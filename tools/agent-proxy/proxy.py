import os
import sys
import uuid
from contextlib import asynccontextmanager
from pathlib import Path

import fastapi
import grpc
import httpx
from fastapi import Request
from fastapi.responses import StreamingResponse

# Generated at launch from the repository's authoritative protobuf definition.
sys.path.insert(0, str(Path(__file__).with_name(".generated")))
import classify_pb2
import classify_pb2_grpc

# Configuration
CLASSIFIER_TARGET = os.getenv("LLM_D_SC_TARGET", "127.0.0.1:50051")
CLASSIFIER_SIGNAL = os.getenv("LLM_D_SC_CLASSIFIER", "complexity")
UPSTREAM_BASE_URL = os.getenv(
    "OPENAI_UPSTREAM_BASE_URL", "https://api.openai.com/v1"
).rstrip("/")
FAST_MODEL = os.getenv("FAST_MODEL", "gpt-5.6-luna")
HEAVY_MODEL = os.getenv("HEAVY_MODEL", "gpt-6-astra")
UPSTREAM_TIMEOUT = httpx.Timeout(
    connect=10.0,
    read=None,
    write=30.0,
    pool=30.0,
)
HOP_BY_HOP_HEADERS = {
    "connection",
    "content-length",
    "host",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
}


@asynccontextmanager
async def lifespan(app: fastapi.FastAPI):
    # Responses streams may be quiet for extended periods while a model reasons.
    # Codex owns the stream-idle policy, so this hop must not impose httpx's
    # short default read timeout.
    app.state.http_client = httpx.AsyncClient(timeout=UPSTREAM_TIMEOUT)
    app.state.classifier_channel = grpc.aio.insecure_channel(CLASSIFIER_TARGET)
    app.state.classifier = classify_pb2_grpc.ClassifyStub(
        app.state.classifier_channel
    )
    yield
    await app.state.classifier_channel.close()
    await app.state.http_client.aclose()


app = fastapi.FastAPI(lifespan=lifespan)


def content_text(content) -> str:
    if isinstance(content, str):
        return content
    if isinstance(content, list):
        return "\n".join(
            part.get("text", "")
            for part in content
            if isinstance(part, dict)
            and part.get("type") in {"text", "input_text", "output_text"}
        )
    return ""


def chat_prompt(body: dict) -> str:
    for message in reversed(body.get("messages", [])):
        if isinstance(message, dict) and message.get("role") == "user":
            return content_text(message.get("content", ""))
    return ""


def responses_prompt(body: dict) -> str:
    response_input = body.get("input", "")
    if isinstance(response_input, str):
        return response_input
    if isinstance(response_input, dict):
        response_input = [response_input]
    if not isinstance(response_input, list):
        return ""

    for item in reversed(response_input):
        if isinstance(item, dict) and item.get("role") == "user":
            return content_text(item.get("content", ""))
    return ""


async def route_request(request: Request, endpoint: str, prompt_reader):
    # 1. Intercept the payload from the coding agent
    body = await request.json()
    prompt = prompt_reader(body)

    # 2. Ask llm-d-sc for a routing decision
    try:
        sc_resp = await request.app.state.classifier.Classify(
            classify_pb2.ClassifyRequest(
                request_id=request.headers.get("x-request-id", str(uuid.uuid4())),
                session_id=request.headers.get("x-session-id", ""),
                context=prompt,
                signals=[CLASSIFIER_SIGNAL],
            ),
            timeout=5,
        )
        classification = sc_resp.ranked[0].label if sc_resp.ranked else "WORK"
    except Exception as e:
        print(f"Classifier failed, defaulting to WORK: {e}")
        classification = "WORK"

    # 3. Modify the requested model based on the semantic triage label
    if classification == "SIMPLE":
        body["model"] = FAST_MODEL
        print(f"🚦 Routing to {FAST_MODEL} ({classification})")
    else:
        body["model"] = HEAVY_MODEL
        print(f"🚦 Routing to {HEAVY_MODEL} ({classification})")

    # 4. Forward the modified request to the real LLM
    headers = {
        name: value
        for name, value in request.headers.items()
        if name.lower() not in HOP_BY_HOP_HEADERS and name.lower() != "content-type"
    }
    headers["content-type"] = "application/json"

    req = request.app.state.http_client.build_request(
        "POST", f"{UPSTREAM_BASE_URL}/{endpoint}", headers=headers, json=body
    )
    response = await request.app.state.http_client.send(req, stream=True)

    async def response_body():
        try:
            async for chunk in response.aiter_bytes():
                yield chunk
        finally:
            await response.aclose()

    # 5. Stream the response directly back to the agent
    response_headers = {
        name: value
        for name, value in response.headers.items()
        if name.lower()
        in {"content-type", "openai-processing-ms", "openai-version", "x-request-id"}
    }
    response_headers.setdefault("content-type", "text/event-stream")
    return StreamingResponse(
        response_body(),
        status_code=response.status_code,
        headers=response_headers,
    )


@app.post("/v1/chat/completions")
async def chat_completions(request: Request):
    return await route_request(request, "chat/completions", chat_prompt)


@app.post("/v1/responses")
async def responses(request: Request):
    return await route_request(request, "responses", responses_prompt)


@app.post("/v1/responses/compact")
async def compact_responses(request: Request):
    return await route_request(request, "responses/compact", responses_prompt)
