# 0036. Keep the access log in daily files the server prunes

Status: Accepted · 2026-11-04

## Context

The access log is kept for security (docs/privacy.md, processing 3): to
detect and date an incident, which the GDPR's 72-hour notification needs.
The privacy notice promises one year at most.

Deployed with Docker, the server wrote it to standard output. Docker keeps
3 × 10 MB of it and deletes it whenever the container is recreated, which
every deploy that changes the server or its settings does. The year was
never exceeded, but the trail was gone after each deploy: an incident
found a week later could not be dated. The deploy guide told operators to
point the log at a file and rotate it themselves, which most would not.

## Decision

- `DRYWIRE_ACCESS_LOG` may name a directory (ending in `/`). The server
  then writes one file per UTC day, `access-YYYY-MM-DD.jsonl`, readable by
  its user only, and deletes each file once its last day is older than
  `DRYWIRE_ACCESS_LOG_DAYS` (365 by default, and at most), at start and at
  each new day. Files with other names are left alone. Standard library
  only.
- The Docker image and the systemd unit point it at the data volume:
  `/data/access/` and `/var/lib/drywire/access/`. Backups, which copy the
  database only, never include it.
- Standard output, a single file and `off` keep working as before.

## Consequences

- The trail survives deploys and restarts, and the retention the privacy
  notice states is enforced by the server, not by an operator's habit.
- A line was 137 bytes in a local run of the image, and a small group
  makes few requests: the volume needs no new care.
- Moving an instance to another host leaves the log behind unless it is
  copied, like the ledger.
