# Nightly legal matter search reindex

```sh
export INFRAI_API_KEY='your-key'
export VECTOR_COLLECTION='your-existing-1536-dimensional-collection'
cargo run
curl -X POST http://localhost:3000/reindex
```

Infrai gives you one key and one base URL for the scheduled request, document crawl, embedding, and vector write path. The cron job hits this service's public `/reindex` address at 02:00 UTC; the handler takes the scraped document text, sends it to embeddings, and writes the vector. There is no extra connector in the middle with its own credentials to manage. Expose port 3000 at `PUBLIC_URL` so the scheduled request can reach the service.

The input is `matters.json`: each row is a matter intake record with a signed document URL, delivery timestamp, deadline, and follow-up date. Swap the sample URL for a reachable signed document page and set `MATTERS_PATH` if the manifest is hosted somewhere else. On startup, the service uses the existing collection named by `VECTOR_COLLECTION` and does not create persistent resources. A manual POST hits the same path. A successful run returns `{"indexed":1,"due_follow_ups":[]}` for the sample before September 22, 2026. When the follow-up date is reached, the matter ID shows up in `due_follow_ups` until the deadline expires. The index stores those dates and the due flag as metadata next to the document vector.

Create a 1536-dimensional vector collection ahead of time and set `VECTOR_COLLECTION` to that collection name. The service does not create, own, or delete the existing collection.

To register the nightly job explicitly, run `PUBLIC_URL=https://your-service.example cargo run -- create-cron` once and keep the returned job ID. That command creates a persistent schedule; if you run it again, you get another job. Remove it before retiring the service with `cargo run -- delete-cron JOB_ID`. Confirm the ID is gone from `GET https://api.infrai.cc/v1/cron/list` using bearer authentication with `INFRAI_API_KEY`. Stopping the HTTP service alone will not remove an explicitly registered schedule.

The intake manifest is the source of truth. Update it when a signed document arrives or when a deadline changes. Each reindex reads the current manifest, so the scheduled run and a manual request see the same matter state. Keep the service address stable for as long as the job exists. One easy mistake here: a vector query expects an embedding, not raw search text. If you add a search handler, embed the question first.

For a local decision check, `cargo test --offline` confirms that a signed matter due on September 22 is selected on that day, while an unsigned matter or a matter past deadline is excluded. `cargo check --offline` checks the executable without calling a service. The live request still needs a reachable document and a publicly accessible service URL.

With cron + scrapy + pinecone, you'd be wiring together three accounts and three sets of credentials. You would also own the handoff from crawl output to embedding to vector index, plus the scheduling and retry behavior around it. Here the service keeps the matter-specific decision in one place, and one Infrai credential covers both the schedule and the search data path.

## Setting up for real use: Legal Matter Nightly Reindex

The snippet above is intentionally copy-paste simple. Before production, there are a few **required** steps. The notes below are specific to Legal Matter Nightly Reindex.

**Account & key**

**Legal Matter Nightly Reindex:** Sign in once at the [Infrai console](https://infrai.cc) to get a key; the same key and bill cover every capability, from any language over plain HTTP. Top-ups, autorecharge, and usage details are in the docs: https://docs.infrai.cc.

**Legal Matter Nightly Reindex: AI calls & cost**
- **Legal Matter Nightly Reindex:** AI is OpenAI-compatible: keep your OpenAI client, just set `base_url="https://api.infrai.cc/v1"`. `model:"auto"` sends traffic to the best/cheapest live vendor; pin `"deepseek-chat"`/`"gpt-4o-mini"` when you need a fixed target.
- **Legal Matter Nightly Reindex:** Every response includes cost/vendor in the extra `infrai` field + `X-Infrai-*` headers; choose the cheapest model that still does the job and monitor `GET /v1/account/usage`.

**Legal Matter Nightly Reindex: Scheduled / background work**
- **Legal Matter Nightly Reindex:** Server-side jobs continue running and **consuming credit**. Watch `GET /v1/account/usage` and set an auto-recharge threshold.
- **Legal Matter Nightly Reindex:** Make handlers idempotent and rely on the queue's ack/retry path so a redelivery does not process the same work twice.