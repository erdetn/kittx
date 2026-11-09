# KittxAnalyzer

Native GPUI Kit UI for analyzing selected data from a `.kittx` ECU recording with
OpenAI, Anthropic, or OpenRouter. The source folder is `src/kittxanalyzer`; the executable
is named **KittxAnalyzer**.

**Analyze sends a synthesized prompt containing only the selected recording
evidence to the chosen service and displays its answer in the report panel.**
The prompt is kept out of the UI and console. The source recording stays read-only.

## Run and build

Use [KittxLauncher](../kittxlauncher/README.md) to open this UI inside its process.
This package retains its standalone binary and exposes `ui::open`, `ui::close`,
and `ui::run` behind the `ui` feature.
Only one Kittx UI may run per user, including direct launches. Close this window
to return to the launcher when started from it.

From the repository root:

```sh
cargo run --locked --manifest-path src/kittxanalyzer/Cargo.toml
cargo build --locked --release --manifest-path src/kittxanalyzer/Cargo.toml
```

Use the [build-and-run script](../../build_analyzer.sh) for a release build with
optional OS selection:

```sh
./build_analyzer.sh              # Build for the host and run
./build_analyzer.sh macos        # Build for macOS, retaining the host CPU
./build_analyzer.sh linux        # Build for Linux GNU, retaining the host CPU
./build_analyzer.sh windows      # Build for Windows MSVC, retaining the host CPU
./build_analyzer.sh --build-only  # Build for the host without launching
./build_analyzer.sh clean        # Remove Cargo build artifacts
```

The script works from any working directory. Use Git Bash or another POSIX shell
on Windows. Without an OS argument it preserves the active compiler's exact host
target, including the Linux libc. Explicit OS selection supports `x86_64` and
`aarch64`. Native builds run after a successful build; targets that differ from
the compiler's host target are built without launching. `--build-only` can appear
before or after the OS argument and prevents launching any build.

Use `clean` by itself to remove the analyzer's Cargo build artifacts without
building or launching. It respects Cargo's configured target directory; recordings
outside that directory are untouched.

Cross-compilation requires the selected Rust target, native libraries, linker,
and OS SDK to be available on the build machine. The script does not install them.
Cargo locates and runs native executables, including when a custom target directory
is configured. The application's exit status is returned by the script.

Script builds put the executable at
`src/kittxanalyzer/target/<target-triple>/release/KittxAnalyzer`
(`KittxAnalyzer.exe` on Windows). On Apple Silicon, for example, the default path
is `src/kittxanalyzer/target/aarch64-apple-darwin/release/KittxAnalyzer`.
The direct Cargo build above, without `--target`, uses
`src/kittxanalyzer/target/release/KittxAnalyzer`. Cargo's target-directory settings
can override these paths.

Like ViewerUI and KittxRecorder, this is an isolated Cargo workspace with GPUI Kit
pinned to 0.7.3 and a Rust 1.92 minimum. It does not add GUI dependencies to the
ELM327 library. See the [viewer build requirements](../viewerui/README.md#build-and-run)
for desktop macOS, Windows, and Linux native dependencies. Mobile is not a UI target.

## AI service setup

Set the key for the service you intend to select, in the environment that launches
the application. For example, in a POSIX shell:

```sh
export OPENAI_API_KEY="your-openai-key"
# Or choose one of these services instead:
export ANTHROPIC_API_KEY="your-anthropic-key"
export OPENROUTER_API_KEY="your-openrouter-key"
./build_analyzer.sh
```

In PowerShell, use `$env:OPENAI_API_KEY = "your-openai-key"` (or the corresponding
variable for the other service), then launch with Cargo or the compiled executable
from that environment. Applications launched from Finder or another desktop
launcher may not inherit a terminal's environment. Relaunch after changing it.
Only the selected service's key is required. Keys are never displayed, printed,
included in the prompt, or saved to the recording.

| Service | Required API key | Default model | Optional model override |
|---|---|---|---|
| OpenAI | `OPENAI_API_KEY` | `gpt-5-mini` | `OPENAI_MODEL` |
| Anthropic | `ANTHROPIC_API_KEY` | `claude-sonnet-4-6` | `ANTHROPIC_MODEL` |
| openrouter.ai (free) | `OPENROUTER_API_KEY` | `nvidia/nemotron-3-ultra-550b-a55b:free` | `OPENROUTER_MODEL` (only `:free` models) |

Use a text model available to your account that supports the selected service's
API. Unset or blank model overrides use the defaults above. Anthropic keys
requiring explicit workspace selection can also use `ANTHROPIC_WORKSPACE_ID`.

OpenRouter uses free inference only. Its default is
[NVIDIA Nemotron 3 Ultra (550B total / 55B active parameters)](https://openrouter.ai/nvidia/nemotron-3-ultra-550b-a55b:free),
the largest free general text model verified as suitable for this single-request
workflow on **2026-10-18**. The larger
[Inkling free endpoint](https://openrouter.ai/thinkingmachines/inkling:free) is
restricted to agentic harnesses. This is a fixed default, not a live model-ranking
algorithm; model size does not guarantee the best answer for a particular task.

OpenRouter model overrides must end in `:free`, for example
`export OPENROUTER_MODEL="nvidia/nemotron-3-super-120b-a12b:free"`.
An existing paid override must be unset or replaced with a free model before
analysis can run. Every OpenRouter request also sets zero prompt, completion,
and per-request prices through
[`provider.max_price`](https://openrouter.ai/docs/guides/routing/provider-selection#max-price).
If free capacity is unavailable or rate limited, the UI shows an error; the
application does not switch to paid inference. The API key is still required,
and free availability and provider/account limits can change.

## Use the UI

The UI uses a 14 px base font and rounded buttons with highlighted outlines in
both light and dark themes, matching ViewerUI.
The dark theme shares KittxProbe's muted navy backgrounds, pastel blue accents,
and cool white text across panels, menus and controls.

1. Click **Open .kittx…** in the header and choose a recording. The existing
   reader validates its format/version; unsupported files show an error.
2. Review the recording/session/vehicle information in the left panel. Measurements
   and DTC items initially start unchecked. Select individual checkboxes or use
   **Select all** / **Clear**.
3. The scrollable measurement list contains all recorded series, retaining the
   signal identifier, unit, ECU/sender identity, and sample count. Numeric series
   appear first; raw/undecoded measurements have their own heading.
4. The DTC list separates codes by category (stored/pending/permanent or the original
   category), namespace, diagnostic profile, ECU, and read outcome. Repeated
   observations of the same item are grouped with a read count. Empty successful
   reads, failed reads, and raw manufacturer replies remain distinct selectable
   items; failures never imply the absence of DTCs. Unknown code descriptions
   are not guessed.
5. Choose an **AI service** and select at least one item to enable **Analyze**.
   Click **Analyze**. The application reads the selected observations, generates
   the prompt internally, and sends it to that service. The button changes to
   **Analyzing…** while waiting. Once the answer is ready, the right panel shows
   the AI report rendered as Markdown, including headings, emphasis, lists,
   tables, inline code, and fenced code blocks. Changing a checkbox or confirming
   a provider choice clears the previous report.
6. Click **Save report…** beside the report title to export the displayed answer
   as a UTF-8 Markdown file. The dialog starts in the recording's directory and
   suggests `<recording-name>-report.md`. The exported content preserves the AI
   report's Markdown and Unicode text; it does not include the input prompt.
7. Drag the divider to resize the panels. The checklist and report scroll
   independently; **Analyze** remains visible below the checklist.
8. The header **Light theme / Dark theme** button switches appearance; dark is
   the startup default. **Exit**, the native window close button, and the Quit menu
   action close the application.

Reports use GPUI Kit's built-in
[`TextView::markdown`](https://gpui-kit.com/component/text-view/) with the active
dark/light theme. Text is selectable, and the report panel keeps its own
scrollbar. **Save report…** exports the original Markdown source.

The **AI service** combobox immediately after **Open .kittx…** offers **Anthropic**,
**OpenAI**, and **openrouter.ai**. It starts without a selected provider and keeps
your choice while the window is open, including across file loads and theme changes.
Analyze stays disabled until a provider and at least one recording item are selected;
the keyboard action follows the same requirements.
Selecting a provider does not contact it. Each press of Analyze makes one request
using that provider and the current selection; prior reports are not sent as history.

Keyboard shortcuts: **Ctrl/Cmd+O** opens, **Ctrl/Cmd+Enter** starts analysis,
and **Ctrl/Cmd+Q** exits. The File menu also provides Open and Exit.

Opening and summarizing the catalog runs off the UI thread. Canceling the picker
or failing to open a new file preserves the current recording and selection.
Loading another file successfully resets the selection, report, and scroll
positions. Data/provider selection, file opening, and Analyze are disabled during
analysis; theme switching and Exit remain available. File-query errors, missing
keys, network failures, invalid responses, authentication failures, rate limits,
and service errors appear in the status bar and restore the controls. Failed
analysis leaves no stale report. If the service returns a partial answer, its
text is shown with an incomplete-report status.

**Save report…** is enabled when a nonempty report is available and the app is
idle. Canceling or failing to save retains the report for another attempt.
During the save dialog/write, data/provider selection, Open, Analyze, and Save
are disabled; theme switching and Exit remain available. Successful saves show
the destination in the status bar. `.md` and `.markdown` extensions are accepted
case-insensitively; other or missing extensions become `.md`. If this changes
the chosen name to an existing file, saving fails without replacing that file.
Select its exact Markdown filename to use the native dialog's overwrite
confirmation. Saving never changes the `.kittx` recording.

The application opens files read-only, uses their committed acquisition cutoff,
and initially loads only catalog metadata. Analyze reads only the selected series
and DTC items in a background task. Reopen to refresh a recording that is still
being written. No migration, repair, recording modification, or ECU communication
occurs.

## Requests and responses

The analyzer uses one shared HTTP implementation (`reqwest` with Rustls TLS) in
the UI's background executor. The bridge library and recorder do not depend on it.

- OpenAI: `POST https://api.openai.com/v1/responses`, bearer authentication,
  `input` text, `store: false`; visible assistant `output_text` and refusals are
  extracted from all message output items.
- Anthropic: `POST https://api.anthropic.com/v1/messages`, `x-api-key` and
  `anthropic-version: 2023-06-01`; assistant text blocks are collected.
- OpenRouter: `POST https://openrouter.ai/api/v1/chat/completions`, bearer
  authentication, a specific `:free` model, `max_tokens`, and zero provider
  price caps; the first choice's assistant text or refusal is displayed.

Requests are non-streaming, with a **15-second connection timeout**, a **180-second
total timeout including the response body**, and **8,192 requested output tokens**.
Reasoning models may spend part of that output allowance on internal reasoning.
Responses larger than **2 MiB** are rejected. Reasoning/thinking blocks and raw
response JSON are not shown. The UI renders returned text and never executes
tools or model instructions.

No automatic retries or redirects occur. A timeout does not establish whether
the provider processed or billed the request; pressing Analyze again starts a
new request. Closing the app stops waiting locally and does not cancel provider
processing already underway. Reports remain in memory until cleared or closed;
**Save report…** exports a separate Markdown copy. Reports are not written into
`.kittx` files. Provider-side handling follows your
provider's account settings and policies; `store: false` is specifically an
OpenAI Responses option.

The wire formats follow the [official OpenAI text-generation documentation](https://developers.openai.com/api/docs/guides/text),
[Anthropic Messages reference](https://platform.claude.com/docs/en/api/messages/create)
and [authentication guide](https://platform.claude.com/docs/en/manage-claude/authentication),
and [OpenRouter chat completion reference](https://openrouter.ai/docs/api/api-reference/chat/create-a-chat-completion).

## Generated prompt

The prompt contains analysis instructions followed by structured JSON evidence:

- Optional `vehicle.brand` and `vehicle.model` provide the recording's stored
  `make` and `model` values as context. Each field is trimmed and included only
  when nonempty. Absent rows, nulls, empty strings, and whitespace-only values are
  omitted; if both fields are missing, the entire `vehicle` object is omitted.
  This context accompanies measurement-only and DTC-only selections as well.
- Each selected measurement series contains its vehicle measurement **label and
  unit**, sample values/timestamps, and useful summaries. Signal IDs, ECU IDs,
  senders, protocols, connection segments, and other transport metadata are
  excluded from the prompt. Original source/unit filters remain internal, and
  separately selected sources stay separate even when their labels match.
- Summary counts, finite min/max, and arithmetic sample mean cover all committed
  observations. Clean numeric samples contain only their observed times and value.
  Null raw fields, default flags, zero quality/omission counts, redundant finite
  counts, and duplicate time-range fields are omitted. Incomplete/partial/truncated
  samples and nonfinite values retain explicit flags/counts when present; missing
  flags do not guarantee data quality. Raw values are never assigned a decoder.
- **Every committed original sample in each selected measurement series** carries
  its original value and monotonic/UTC timestamps, in chronological order with row
  ID breaking timestamp ties. Repeated values are preserved. There is no sample
  cap, downsampling, interpolation, or truncation of raw measurement values.
  The arithmetic mean is not a time-weighted trip average. Long recordings produce
  larger prompts; service input limits may cause a request to fail, and the analyzer
  does not silently shorten the selected measurement data to fit.
- Each selected DTC code/category/namespace/profile/ECU/outcome retains its total
  read count and up to **128 original timestamped reads**, spread across its history.
  Omitted reads are counted. Nullable ECU identity, empty successful scans, failed
  scans, and raw manufacturer replies remain distinct. Duplicate code entries in
  one read use the first original entry, with their matching-entry count stated.
- Selected raw measurement payloads are included in full. Raw diagnostic replies
  and code/status bytes include at most **128 bytes per field**, with omitted-byte
  counts. A selected
  decoded DTC code includes its own code/status bytes, never the complete ECU reply
  or other codes from the same scan.

Both time axes use the file's original integer microseconds. The prompt supplies
the elapsed recording origin so relative times can be calculated. A DTC timestamp
indicates when it was observed, not when the fault began. Failed reads never imply
an empty code set.

Only selected measurements/DTC items and available vehicle brand/model enter
the prompt. VIN, other vehicle fields, metadata provenance, filenames/paths,
unselected signals/codes, and whole wire transactions are excluded. Recording
strings are JSON-quoted data, and the instructions explicitly separate evidence
from hypotheses and prohibit invented unknown decoders or vendor descriptions.
This evidence is submitted only to the service selected when Analyze is pressed.

## Validation and coverage

```sh
sh -n build_analyzer.sh
python3 -B -m unittest discover -s tests -p test_build_analyzer.py
cargo fmt --manifest-path src/kittxanalyzer/Cargo.toml --check
cargo test --locked --manifest-path src/kittxanalyzer/Cargo.toml --features ui-test
cargo test --locked --manifest-path src/kittxanalyzer/Cargo.toml --no-default-features
cargo clippy --locked --manifest-path src/kittxanalyzer/Cargo.toml --all-targets --features ui-test -- -D warnings
cargo clippy --locked --manifest-path src/kittxanalyzer/Cargo.toml --all-targets --no-default-features -- -D warnings
cargo build --locked --manifest-path src/kittxanalyzer/Cargo.toml
```

| API / behavior | Coverage |
|---|---|
| `Recording::open`, `Recording` metadata, `RecordedDtc` identity/count/times, `RecordedDtc::label` | `tests/recordings.rs`: catalog_retains_ecu_category_namespace_profile_and_read_outcomes; empty_recordings_missing_files_wrong_types_and_versions |
| `Selection::{default,all}`, `Selection` sets, `Recording::selection_preview`, empty/invalid selections, raw-data boundaries | `tests/recordings.rs`: preview_uses_only_selected_data_and_never_claims_analysis; catalog test |
| `signal_label`, known measurements and unknown IDs | `tests/recordings.rs`: labels_cover_decoded_measurements_and_preserve_unknown_identifiers |
| `Recording::analysis_prompt`: all selected original measurement values/times and raw bytes, chronological order/ties, repeated values, compact fields, independent sources, complete summaries, bounded DTC detail, quality, snapshots, nonfinite/quoted data, invalid inputs | `tests/prompts.rs` |
| Complete large measurement prompts reach every provider unchanged | `ai_tests.rs`: providers_send_large_measurement_prompts_without_truncation (local HTTP peers, 4097 samples per request) |
| Optional vehicle brand/model context for all selection types; independent omission of absent/null/empty/blank fields, trimming, quoted custom values, read-only behavior, exclusion of VIN/unrelated metadata | `tests/prompts.rs`: vehicle_brand_and_model_are_included_independently_only_when_present; vehicle_brand_and_model_remain_quoted_recording_data |
| `AiService::{ALL,label,api_key_env,model_env,default_model}`, every provider variant, endpoint/header/body contracts and `AnalysisReport` text/completeness | `ai_tests.rs`: providers_send_their_own_headers_models_and_request_formats; provider response parser tests |
| Environment lookup, optional models/workspace, missing/blank/invalid keys, malformed model values, public `ai::analyze` empty-input rejection | `ai_tests.rs`: environment_overrides_and_workspace_are_scoped_to_the_selected_provider; configuration_rejects_missing_empty_nonunicode_and_invalid_values_without_exposing_them |
| Visible answers, refusals, reasoning exclusion, partial generations, empty/malformed/error responses | `ai_tests.rs`: openai_collects_only_visible_assistant_text_and_preserves_incomplete_answers_and_refusals; anthropic_text_blocks_exclude_thinking_and_mark_interrupted_generations; openrouter_handles_text_blocks_refusals_and_finish_reasons_without_showing_reasoning; malformed_failed_empty_and_unexpected_provider_responses_are_errors |
| HTTP status failures/redaction, no retries/credential forwarding, response-body limits, connection failures, header/body total deadlines | `ai_tests.rs`: http_errors_do_not_expose_provider_bodies_or_retry_requests; redirects_are_not_followed_and_do_not_forward_credentials; malformed_bodies_provider_errors_and_response_limits_are_handled; connection_failure_and_total_deadlines_cover_headers_and_slow_response_bodies |
| OpenRouter free default and overrides, rejected paid/automatic routes, zero prompt/completion/request prices on the actual HTTP request, no model fallback list | `ai_tests.rs`: providers_send_their_own_headers_models_and_request_formats; openrouter_rejects_paid_or_automatic_overrides_and_keeps_free_overrides_at_zero_price |
| Every `AnalysisError` variant, `Display`, `Debug`, standard error interface | `ai_tests.rs`: errors_are_displayable_without_raw_credentials_requests_or_sources; configuration/HTTP/parser tests |
| Picker load/cancel, checkbox clicks, select all/clear, disabled controls, AI feedback display, hidden prompt, default dark and theme toggle, narrow layout, Exit | `ui_tests.rs`: empty_state_theme_toggle_disabled_analyze_picker_cancel_and_exit; file_picker_selected_evidence_ai_report_and_both_theme_layouts |
| Failed-open retention, new-file reset, empty recording UI | `ui_tests.rs`: failed_load_preserves_previous_file_and_successful_reopen_resets_selection |
| Long checklist, DTC selection after scrolling, independent Markdown report scrolling, report scroll reset; headings, emphasis, lists, tables, inline/fenced code in report/export fixtures and both-theme narrow layouts | `ui_tests.rs`: long_checklist_and_report_scroll_independently; save_report_cancel_export_and_errors_preserve_feedback_and_recording |
| Prompt-generation failure unlocks controls without a stale report | `ui_tests.rs`: prompt_generation_failure_unlocks_controls_without_a_report |
| AI service combobox, all three choices, keyboard confirmation/cancel, selection retention, header order and right-aligned theme/Exit at normal/minimum widths in both themes | `ui_tests.rs`: ai_service_choices_and_header_layout |
| Analyze requires a service and selected data; disabled clicks and action dispatch cannot start work without a service or duplicate an active request | `ui_tests.rs`: file_picker_selected_evidence_ai_report_and_both_theme_layouts; analyze_uses_the_selected_provider_and_hides_the_prompt |
| Provider routing, exact selected evidence sent, report invalidation, theme switching during work, errors unlock controls, partial feedback retained | `ui_tests.rs`: analyze_uses_the_selected_provider_and_hides_the_prompt; ai_failures_unlock_controls_and_incomplete_feedback_is_visible |
| Save button/action gating, native dialog cancellation/directory, UTF-8 Markdown round trip, busy/reentrant actions, successful overwrite, write-error retention/retry, report invalidation, both themes/minimum widths, read-only source | `ui_tests.rs`: save_report_cancel_export_and_errors_preserve_feedback_and_recording |
| Markdown extension normalization, accepted case-insensitive extensions, existing normalized-target preservation, invalid paths, source-file protection | `ui_tests.rs`: markdown_export_extensions_and_existing_targets_do_not_modify_the_recording |
| Release build and launch, host/OS/architecture selection, build-only and foreign-target behavior, standalone clean command and failure propagation, rejected clean combinations, quoted paths, working-directory independence, help, failures and exit statuses | `tests/test_build_analyzer.py` (repository root; fake tools on POSIX hosts) |

Tests use local recording fixtures, loopback HTTP servers, fake AI replies, and
headless GPUI windows. No hardware, API keys, external network, or paid LLM calls
are required. Script tests use fake compiler/Cargo tools;
they do not install SDKs, cross-compile GPUI, or open a GUI. Desktop CI checks
macOS, Windows, and Linux; local native compilation and headless UI tests do not
establish visual acceptance on other operating systems.
