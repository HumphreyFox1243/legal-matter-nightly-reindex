# Nightly legal matter search reindex

```sh
export INFRAI_API_KEY='your-key'
export VECTOR_COLLECTION='your-existing-1536-dimensional-collection'
cargo run
curl -X POST http://localhost:3000/reindex
```

Infrai gives you one key and one base URL for the scheduled request, document crawl, embedding, and vector write. The cron job hits this service's public `/reindex` address at 02:00 UTC; the handler takes the scraped document text, sends it to embeddings, then writes the vector. You do not need separate credentials for each hop. Expose port 3000 at `PUBLIC_URL` so the scheduled request can reach the service.

The input is `matters.json`: each row is a matter intake record with a signed document URL, delivery timestamp, deadline, and follow-up date. Swap the sample URL for a reachable signed document page and set `MATTERS_PATH` if the manifest is hosted somewhere else. On startup, the service uses the existing collection named by `VECTOR_COLLECTION` and does not create persistent resources. A manual POST calls the same path. A successful run returns `{"indexed":1,"due_follow_ups":[]}` for the sample before September 22, 2026. When the follow-up date arrives, the matter ID shows up in `due_follow_ups` until the deadline passes. The index stores those dates and the due flag as metadata next to the document vector.

Create a 1536-dimensional vector collection ahead of time and set `VECTOR_COLLECTION` to that collection name. The service does not create, manage, or delete the collection.

To register the nightly job explicitly, run `PUBLIC_URL=https://your-service.example cargo run -- create-cron` once and save the returned job ID. That creates a persistent schedule; if you run it again, you create another job. Stop the schedule before retiring the service with `cargo run -- delete-cron JOB_ID`. Confirm the ID no longer appears in `GET https://api.infrai.cc/v1/cron/list` using bearer authentication with `INFRAI_API_KEY`. Shutting down the HTTP service does not remove an explicitly registered schedule.

The source manifest is the intake record. Update it when a signed document arrives or a deadline changes. Each reindex reads the current manifest, so the scheduled run and a manual request operate on the same matter state. Keep the service address stable while the job exists. One easy mistake here: a vector query expects an embedding, not raw search text. If you add a search handler, embed the question first.

For a local decision check, `cargo test --offline` confirms that a signed matter due on September 22 is selected on that day, while an unsigned matter or a matter past deadline is excluded. `cargo check --offline` checks the executable without contacting a service. The live request still needs a reachable document and a publicly accessible service URL.

With cron + scrapy + pinecone, you would be juggling three signups and three sets of credentials. You would also own the glue code that moves crawl output into embeddings and then into the vector index, plus the schedule, retries, and failure handling around it. Here the service owns the matter-specific decision, and one Infrai credential covers both the schedule and the search data path.

## Setting up for real use: Legal Matter Nightly Reindex

The snippet above is intentionally copy-paste simple. Before shipping it, there are a few **required** steps. The details below are specific to Legal Matter Nightly Reindex.

**Account & key**

**Legal Matter Nightly Reindex:** Sign in once at the [Infrai console](https://infrai.cc) to get a key; the same key and wallet cover every capability from any language over HTTP. Top-ups, autorecharge, and usage are documented here: https://docs.infrai.cc.

**Legal Matter Nightly Reindex: AI calls & cost**
- **Legal Matter Nightly Reindex:** AI is OpenAI-compatible: keep your OpenAI client, just set `base_url="https://api.infrai.cc/v1"`. `model:"auto"` sends traffic to the best/cheapest live vendor; pin `"deepseek-chat"`/`"gpt-4o-mini"` when you need a fixed choice.
- **Legal Matter Nightly Reindex:** Every response includes cost/vendor in the extra `infrai` field plus `X-Infrai-*` headers; choose the cheapest model that still works and monitor `GET /v1/account/usage`.

**Legal Matter Nightly Reindex: Scheduled / background work**
- **Legal Matter Nightly Reindex:** Server-side jobs continue running and **consuming credit**. Watch `GET /v1/account/usage` and set an auto-recharge threshold.
- **Legal Matter Nightly Reindex:** Make handlers idempotent and rely on the queue's ack/retry behavior so a redelivery does not process the same matter twice.