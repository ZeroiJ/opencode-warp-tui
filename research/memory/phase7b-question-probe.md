# Phase 7B — Live Question-Delivery Probe (resolves 7A-R2)

> Probe conducted live against the managed OpenCode server before any
> Phase 7B code. Resolves decision-log **7A-R2** with hard evidence.
> Server: OpenCode **2.0.8**, `http://127.0.0.1:49374` (PID 4922),
> location `/home/zeroij`, Basic auth from `service.json` (never printed).
> Probe modules: `/tmp/probe_question.py`, `/tmp/probe_lifecycle.py`;
> evidence logs: `/tmp/owt-probe-events.jsonl`, `/tmp/owt-probe-lifecycle.jsonl`.

## Outcome

**Candidate B verified — the answer mechanism is the documented form-reply
route.** On live 2.0.8 the `question` tool call does **not** produce a
`question.v2.asked` event (that event never fires on this server); instead
the server materializes the question as a **form** (`frm_`) and emits
`form.created`. Answering is `POST /api/session/{sid}/form/{formID}/reply`
with `{"answer": {"q0": "Dark"}}` → **204 No Content**, and the agent
demonstrably receives the choice (tool state `completed` with
`metadata.answers: [["Dark"]]` and content *"User has answered your
questions: "…"="Dark". You can now continue…"*), then
`session.execution.succeeded`.

## Evidence trail (both throwaway sessions deleted)

1. Created throwaway session `ses_f45e38a80ffelQoq3RJe77BJzU` (later
   deleted, 204; confirmed absent from the session list).
2. Sent a prompt forcing the `question` (AskUserQuestion) tool.
3. Message history shows the pending tool call:

   ```json
   {"type":"tool","id":"call_1efa843717a347f4a146af02","name":"question",
    "executed":false,"state":{"status":"running","input":{"questions":[
      {"question":"Which color scheme do you prefer?","header":"Color scheme",
       "options":[{"label":"Dark",...},{"label":"Light",...},{"label":"Solarized",...}]}]}}}
   ```

4. SSE emitted **`form.created`** (see exact envelope in
   `/tmp/owt-probe-events.jsonl`):

   ```json
   {"data":{"form":{"id":"frm_0ba1c9bb4001NQbQT815EjAB70",
     "sessionID":"ses_f45e...","title":"Questions",
     "metadata":{"kind":"question","tool":{"messageID":"msg_...","id":"call_..."}},
     "fields":[{"key":"q0","title":"Color scheme",
       "description":"Which color scheme do you prefer?","type":"string",
       "options":[{"value":"Dark","label":"Dark",...},
                  {"value":"Light","label":"Light",...},
                  {"value":"Solarized","label":"Solarized",...}],
       "custom":true}]}}}
   ```

5. **Answer delivered** via the documented route:

   ```text
   POST /api/session/ses_f45e.../form/frm_0ba1c9bb4001NQbQT815EjAB70/reply
   body {"answer": {"q0": "Dark"}}        → 204
   ```

6. Delivery **proven**: after the reply, `GET .../message` shows the tool
   object now `state.status: "completed"`, `state.content[0].text` =
   *"User has answered your questions: "Which color scheme do you
   prefer?"="Dark". You can now continue with the user's answers in
   mind."*, `state.metadata.answers: [["Dark"]]`, and a follow-up
   assistant message *"You chose **Dark**..."*; a second cycle (q0=Banan)
   confirmed repeatability and produced the SSE lifecycle:

   ```text
   session.execution.started
   form.created          {form:{id:frm_..., metadata:{kind:"question",...}, fields:[...]}}
   form.replied          {id:frm_..., sessionID:..., answer:{q0:"Banana"}}
   session.execution.succeeded
   ```

## Implications for the adapter

- The `question.v2.asked`/`question.v2.replied`/`question.v2.rejected`
  events never fire on live 2.0.8. The current mapper/state listening for
  them is a dead path on this server. The live question arrives as
  `form.created` with `metadata.kind == "question"`.
- Answer routing needs: the **form id** (`frm_`), the **field key**
  (`q0`), and the **option value** (form `options[].value`, which happens
  to equal `label` in the probe but is a distinct field). The `Question`
  block today carries only labels, so the adapter must track
  form-id/field-key/option-values alongside the rendered gate.
- Reply body is `{"answer": {"q0": "Dark"}}` → 204; errors would surface
  as 400 `FormInvalidAnswerError` (in-band Error/Notice, per O-A).
- History: a *completed* question tool call carries `state.status:
  "completed"`, `state.metadata.answers`, and the
  "User has answered your questions…" summary — the history mapper should
  render answered questions as completed (non-gate) states.

## O-B findings (same probe run)

- Message pagination is **cursor-based**, verified live:
  `GET /api/session/{sid}/message?limit=&order=&cursor=&type=` returns
  `{data, cursor:{previous,next}}`. **No `offset`** param exists on
  2.0.8; the response schema names `limit`/`order`/`cursor`/`type` (all
  optional). `order=asc`/`order=desc` verified; cursor follow works.
- **Default order is `desc` (newest first)** — confirmed by created
  timestamps. `hydrate_active` today fetches `?limit=200` with the default
  order and maps in wire order → the transcript would render newest-first
  (inverted). Fix: request `order=asc` and page `cursor.next` until
  exhausted (with a page cap) so blocks are oldest→newest.
- Session list pagination also cursor-based; not needed for 7B.

## Permission surface (same probe)

- `Permission.Reply` = `once | always | reject` (live OpenAPI). The
  adapter keeps `once`/`reject`; `always` stays unsurfaced per 7A-R5
  security ("no silent persistent grants").

## Cleanup / hygiene

- Throwaway sessions deleted; `/home/zeroij/warp` untouched; server
  version/port/service.json unchanged; the live session
  `ses_f4f7ed847ffeoRPrdgXUr7wlIR` (this conversation's session) was only
  read (message listing probes), never written.