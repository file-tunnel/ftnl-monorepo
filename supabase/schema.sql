-- File Tunnel client telemetry — declarative Supabase schema (dpm-style).
--
-- Org-wide requirement: every File Tunnel client (the WASM/TypeScript upload
-- portal + web UI components via supabase-js, the Dart/Flutter UI via
-- supabase_flutter) keeps a bounded, redacted ring buffer of log levels + device
-- info and streams it DIRECTLY into Supabase — no app-server hop — via the
-- SECURITY DEFINER ingest RPCs below. The read side (this MCP server's
-- client_log_* tools) uses the service-role key over PostgREST.
--
-- CRITICAL redaction invariant: pairing secrets, desktop/phone capabilities,
-- event tickets, presigned URLs, and file bytes are NEVER part of telemetry —
-- only display-safe metadata (job/tunnel/file IDs, lifecycle state, redacted
-- reason codes). Clients enforce this before flushing; the schema stores whatever
-- the RPCs receive, so keep the client sinks honest.
--
-- This is dpm-managed (github.com/declarative-migrations): declarative,
-- stateless, ORM-agnostic — describe the desired end state, no tracked
-- migration files. Idempotent so `dpm apply` can converge.

-- ── snapshots: one row per flushed client ring buffer ──────────────────────

CREATE TABLE IF NOT EXISTS "public"."ftnl_client_log_snapshots" (
  "id"                     uuid PRIMARY KEY DEFAULT gen_random_uuid() NOT NULL,
  "user_id"                uuid,                         -- null for anonymous
  "session_id"             varchar(100),
  "log_entries_json"       text,                         -- the buffered entries
  "log_entries_count"      integer DEFAULT 0 NOT NULL,
  "log_entries_size_bytes" integer DEFAULT 0 NOT NULL,
  "trigger"                jsonb DEFAULT '{}'::jsonb NOT NULL,
  "context"                jsonb DEFAULT '{}'::jsonb NOT NULL,
  "trace_id"               varchar(150),
  "commit_id"              varchar(60),
  "environment"            varchar(30) DEFAULT 'unknown' NOT NULL,
  "client_kind"            varchar(30) DEFAULT 'web' NOT NULL,  -- web | flutter
  "meta"                   jsonb DEFAULT '{}'::jsonb NOT NULL,
  "snapshot_taken_at"      timestamptz DEFAULT now() NOT NULL,
  "created_at"             timestamptz DEFAULT now() NOT NULL,
  "is_soft_deleted"        boolean DEFAULT false NOT NULL
);

CREATE INDEX IF NOT EXISTS "ftnl_cls_session_id_idx"        ON "public"."ftnl_client_log_snapshots" ("session_id");
CREATE INDEX IF NOT EXISTS "ftnl_cls_env_idx"               ON "public"."ftnl_client_log_snapshots" ("environment");
CREATE INDEX IF NOT EXISTS "ftnl_cls_trace_id_idx"          ON "public"."ftnl_client_log_snapshots" ("trace_id");
CREATE INDEX IF NOT EXISTS "ftnl_cls_snapshot_taken_at_idx" ON "public"."ftnl_client_log_snapshots" ("snapshot_taken_at");

-- ── entries: individual streamed log lines ─────────────────────────────────

CREATE TABLE IF NOT EXISTS "public"."ftnl_client_log_entries" (
  "id"               uuid PRIMARY KEY DEFAULT gen_random_uuid() NOT NULL,
  "user_id"          uuid,
  "session_id"       varchar(100),
  "trace_id"         varchar(150),
  "commit_id"        varchar(60),
  "environment"      varchar(30) DEFAULT 'unknown' NOT NULL,
  "level"            varchar(20) DEFAULT 'info' NOT NULL,   -- error|warn|info|debug|trace
  "message"          text NOT NULL,
  "stack"            text,
  "url"              text,
  "source"           varchar(50) DEFAULT 'browser' NOT NULL,  -- browser | portal | flutter | native
  "category"         varchar(50),
  "metadata"         jsonb DEFAULT '{}'::jsonb NOT NULL,
  "client_timestamp" timestamptz NOT NULL,
  "created_at"       timestamptz DEFAULT now() NOT NULL,
  "is_soft_deleted"  boolean DEFAULT false NOT NULL
);

CREATE INDEX IF NOT EXISTS "ftnl_cle_session_id_idx"       ON "public"."ftnl_client_log_entries" ("session_id");
CREATE INDEX IF NOT EXISTS "ftnl_cle_env_idx"              ON "public"."ftnl_client_log_entries" ("environment");
CREATE INDEX IF NOT EXISTS "ftnl_cle_level_idx"            ON "public"."ftnl_client_log_entries" ("level");
CREATE INDEX IF NOT EXISTS "ftnl_cle_client_timestamp_idx" ON "public"."ftnl_client_log_entries" ("client_timestamp");

-- ── Row Level Security: writes only through the ingest RPCs ────────────────
--
-- The RPCs below are the ONLY client write path, which is what every client
-- actually uses (`supabase.rpc('ingest_ftnl_client_log_entries', …)` in the
-- TypeScript, Dart, and Flutter sinks). The tables themselves grant nothing to
-- anon/authenticated: granting direct INSERT would make the RPCs' guardrails
-- (the 600 KB snapshot ceiling, the 500-row batch ceiling, the user_id
-- derivation, and the level/environment defaults) advisory rather than enforced.
-- The RPCs are SECURITY DEFINER and run as owner, so revoking table access does
-- not affect them.
--
-- Reads stay closed to anon/authenticated entirely: there is no SELECT policy,
-- and the MCP server's client_log_* tools read with the service-role key.

ALTER TABLE "public"."ftnl_client_log_snapshots" ENABLE ROW LEVEL SECURITY;
ALTER TABLE "public"."ftnl_client_log_entries"   ENABLE ROW LEVEL SECURITY;

REVOKE ALL ON TABLE "public"."ftnl_client_log_snapshots" FROM anon, authenticated;
REVOKE ALL ON TABLE "public"."ftnl_client_log_entries"   FROM anon, authenticated;

DROP POLICY IF EXISTS "ftnl_service_all_snapshots" ON "public"."ftnl_client_log_snapshots";
DROP POLICY IF EXISTS "ftnl_service_all_entries"   ON "public"."ftnl_client_log_entries";

CREATE POLICY "ftnl_service_all_snapshots" ON "public"."ftnl_client_log_snapshots" FOR ALL TO service_role USING (true) WITH CHECK (true);
CREATE POLICY "ftnl_service_all_entries"   ON "public"."ftnl_client_log_entries"   FOR ALL TO service_role USING (true) WITH CHECK (true);

-- ── ingest RPC: snapshot (single JSONB payload for a clean client API) ─────
-- Called from the browser/Flutter client:
--   supabase.rpc('ingest_ftnl_client_log_snapshot', { payload: {...} })

CREATE OR REPLACE FUNCTION public.ingest_ftnl_client_log_snapshot(payload jsonb)
RETURNS jsonb
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = public
AS $$
DECLARE
  v_payload jsonb := COALESCE(payload, '{}'::jsonb);
  v_entries_json text := v_payload->>'log_entries_json';
  -- Identity comes from the verified JWT, never from the payload. These RPCs are
  -- callable with the public anon key, so a caller-supplied user_id would let
  -- anyone attribute telemetry to any user — and this is the trail an operator
  -- reads through the MCP client_log_* tools when investigating an incident.
  v_user_id uuid := auth.uid();
  v_claimed_user_id text := NULLIF(v_payload->>'user_id', '');
  v_snapshot_id uuid;
BEGIN
  -- 600KB guardrail (JSON overhead headroom).
  IF v_entries_json IS NOT NULL AND octet_length(v_entries_json) > 600000 THEN
    RETURN jsonb_build_object('success', false, 'error', 'log entries exceed 600KB limit');
  END IF;

  INSERT INTO public.ftnl_client_log_snapshots (
    user_id, session_id, log_entries_json,
    log_entries_count, log_entries_size_bytes,
    trigger, context, trace_id, commit_id, environment, client_kind,
    snapshot_taken_at, meta
  ) VALUES (
    v_user_id,
    NULLIF(v_payload->>'session_id', ''),
    v_entries_json,
    COALESCE((NULLIF(v_payload->>'log_entries_count',''))::integer, 0),
    COALESCE((NULLIF(v_payload->>'log_entries_size_bytes',''))::integer,
             COALESCE(octet_length(v_entries_json), 0)),
    COALESCE(v_payload->'trigger', '{}'::jsonb),
    COALESCE(v_payload->'context', '{}'::jsonb),
    NULLIF(v_payload->>'trace_id', ''),
    NULLIF(v_payload->>'commit_id', ''),
    COALESCE(NULLIF(v_payload->>'environment', ''), 'unknown'),
    COALESCE(NULLIF(v_payload->>'client_kind', ''), 'web'),
    COALESCE((NULLIF(v_payload->>'snapshot_taken_at',''))::timestamptz, now()),
    COALESCE(v_payload->'meta', '{}'::jsonb) ||
      jsonb_build_object('directToSupabase', true, 'ingestTransport', 'supabase-rpc', 'rpcVersion', 1) ||
      -- Keep an unverified claim for debugging, clearly separated from user_id so
      -- it can never be mistaken for an authenticated identity.
      CASE
        WHEN v_claimed_user_id IS NOT NULL
         AND (v_user_id IS NULL OR v_claimed_user_id <> v_user_id::text)
        THEN jsonb_build_object('claimedUserIdUnverified', v_claimed_user_id)
        ELSE '{}'::jsonb
      END
  ) RETURNING id INTO v_snapshot_id;

  RETURN jsonb_build_object('success', true, 'snapshot_id', v_snapshot_id::text);
EXCEPTION WHEN OTHERS THEN
  RETURN jsonb_build_object('success', false, 'error', SQLERRM, 'code', SQLSTATE);
END;
$$;

-- ── ingest RPC: batch entries ──────────────────────────────────────────────
--   supabase.rpc('ingest_ftnl_client_log_entries', { entries: [...] })

CREATE OR REPLACE FUNCTION public.ingest_ftnl_client_log_entries(entries jsonb)
RETURNS jsonb
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = public
AS $$
DECLARE
  v_entries jsonb := COALESCE(entries, '[]'::jsonb);
  v_count integer := 0;
BEGIN
  IF jsonb_typeof(v_entries) <> 'array' THEN
    RETURN jsonb_build_object('success', false, 'error', 'entries must be a JSON array');
  END IF;
  IF jsonb_array_length(v_entries) > 500 THEN
    RETURN jsonb_build_object('success', false, 'error', 'maximum 500 entries per call');
  END IF;

  INSERT INTO public.ftnl_client_log_entries (
    user_id, session_id, trace_id, commit_id, environment,
    level, message, stack, url, source, category, metadata, client_timestamp
  )
  SELECT
    -- Verified identity only; see the snapshot RPC. A caller-supplied user_id is
    -- ignored rather than trusted, so a batch sent with the public anon key
    -- cannot attribute entries to another user.
    auth.uid(),
    NULLIF(e->>'session_id', ''),
    NULLIF(e->>'trace_id', ''),
    NULLIF(e->>'commit_id', ''),
    COALESCE(NULLIF(e->>'environment', ''), 'unknown'),
    COALESCE(NULLIF(e->>'level', ''), 'info'),
    COALESCE(NULLIF(e->>'message', ''), '[empty message]'),
    NULLIF(e->>'stack', ''),
    NULLIF(e->>'url', ''),
    COALESCE(NULLIF(e->>'source', ''), 'browser'),
    NULLIF(e->>'category', ''),
    CASE WHEN jsonb_typeof(e->'metadata') IN ('array','object')
      THEN COALESCE(e->'metadata', '{}'::jsonb) ELSE '{}'::jsonb END,
    CASE WHEN COALESCE(e->>'client_timestamp','') ~ '^\d{4}-\d{2}-\d{2}T'
      THEN (e->>'client_timestamp')::timestamptz ELSE now() END
  FROM jsonb_array_elements(v_entries) AS e;

  GET DIAGNOSTICS v_count = ROW_COUNT;
  RETURN jsonb_build_object('success', true, 'inserted', v_count);
EXCEPTION WHEN OTHERS THEN
  RETURN jsonb_build_object('success', false, 'error', SQLERRM, 'code', SQLSTATE, 'inserted', v_count);
END;
$$;

GRANT EXECUTE ON FUNCTION public.ingest_ftnl_client_log_snapshot(jsonb) TO anon, authenticated, service_role;
GRANT EXECUTE ON FUNCTION public.ingest_ftnl_client_log_entries(jsonb)  TO anon, authenticated, service_role;

-- ── Realtime publication so dashboards can also subscribe over websockets ──

DO $$
BEGIN
  BEGIN
    ALTER PUBLICATION supabase_realtime ADD TABLE public.ftnl_client_log_snapshots;
  EXCEPTION
    WHEN duplicate_object THEN RAISE NOTICE 'snapshots already in supabase_realtime';
    WHEN undefined_object THEN
      CREATE PUBLICATION supabase_realtime;
      ALTER PUBLICATION supabase_realtime ADD TABLE public.ftnl_client_log_snapshots;
  END;
  BEGIN
    ALTER PUBLICATION supabase_realtime ADD TABLE public.ftnl_client_log_entries;
  EXCEPTION
    WHEN duplicate_object THEN RAISE NOTICE 'entries already in supabase_realtime';
    WHEN undefined_object THEN
      CREATE PUBLICATION supabase_realtime;
      ALTER PUBLICATION supabase_realtime ADD TABLE public.ftnl_client_log_entries;
  END;
END;
$$;
