use super::store::{Pending, Store};
use super::*;
use std::{path::Path, time::Duration};

/// Opt-in application-side orchestrator. No I/O in constructors, threads or queues.
/// Completion and progress callbacks run inline on the invoking thread.
pub struct Recorder<S: DataSource, C: Clock = SystemClock> {
    source: S,
    clock: C,
    config: RecordingConfig,
    store: Option<Store>,
    state: RecordingState,
    progress: RecordingProgress,
    pending: Vec<Pending>,
    pending_bytes: usize,
    sequence: i64,
    last_time: Option<Timestamp>,
    active_start: i64,
    next_signals: Vec<i64>,
    next_scan: i64,
    connected: bool,
    poisoned: bool,
}
impl<S: DataSource> Recorder<S, SystemClock> {
    pub fn new(source: S, config: RecordingConfig) -> DataResult<Self> {
        Self::with_clock(source, config, SystemClock::default())
    }
}
impl<S: DataSource, C: Clock> Recorder<S, C> {
    pub fn with_clock(source: S, config: RecordingConfig, clock: C) -> DataResult<Self> {
        config.validate()?;
        Ok(Self {
            source,
            clock,
            config,
            store: None,
            state: RecordingState::New,
            progress: RecordingProgress::default(),
            pending: Vec::new(),
            pending_bytes: 0,
            sequence: 0,
            last_time: None,
            active_start: 0,
            next_signals: Vec::new(),
            next_scan: 0,
            connected: false,
            poisoned: false,
        })
    }
    pub fn state(&self) -> RecordingState {
        self.state
    }
    pub fn config(&self) -> &RecordingConfig {
        &self.config
    }
    pub fn progress(&self) -> &RecordingProgress {
        &self.progress
    }
    pub fn session_id(&self) -> Option<&str> {
        self.store.as_ref().map(|s| s.id.as_str())
    }
    pub fn start(
        &mut self,
        path: impl AsRef<Path>,
        complete: impl FnOnce(DataResult<RecordingProgress>),
    ) {
        let result = self.start_inner(path.as_ref());
        self.complete(result, complete);
    }
    pub fn capture_signal(
        &mut self,
        index: usize,
        complete: impl FnOnce(DataResult<RecordingProgress>),
    ) {
        let result = (|| {
            self.require_recording()?;
            let s = self
                .config
                .signals
                .get(index)
                .ok_or_else(|| invalid("signal index out of range"))?
                .clone();
            let time = self.now()?;
            self.capture(
                ReadRequest::Signal(s.signal),
                s.target,
                None,
                time.elapsed_us,
                self.config.operation_timeout,
            )?;
            Ok(self.progress.clone())
        })();
        self.complete(result, complete);
    }
    pub fn scan_dtcs(
        &mut self,
        trigger: ScanTrigger,
        complete: impl FnOnce(DataResult<RecordingProgress>),
    ) {
        let result = (|| {
            self.require_recording()?;
            self.scan(trigger, None)?;
            Ok(self.progress.clone())
        })();
        self.complete(result, complete);
    }
    pub fn flush(&mut self, complete: impl FnOnce(DataResult<RecordingProgress>)) {
        let result = self.flush_inner().map(|()| self.progress.clone());
        self.complete(result, complete);
    }
    /// Consistent live export; the original stays open and recording.
    pub fn snapshot(&mut self, path: impl AsRef<Path>, complete: impl FnOnce(DataResult<()>)) {
        let result = (|| {
            self.require_recording()?;
            self.flush_inner()?;
            let time = self.now()?;
            self.db()?.snapshot(path.as_ref(), time)
        })();
        self.complete(result, complete);
    }
    /// Explicit reconnect only. A different VIN fails and is preserved as evidence.
    pub fn reconnect(&mut self, complete: impl FnOnce(DataResult<RecordingProgress>)) {
        let result = (|| {
            if self.poisoned || self.store.is_none() || self.state == RecordingState::Finished {
                return Err(DataError::State("cannot reconnect this recording".into()));
            }
            let conflict: bool = self.db()?.db()?.query_row(
                "SELECT vin_status='conflict' FROM vehicle",
                [],
                |r| r.get(0),
            )?;
            if conflict {
                return Err(DataError::State(
                    "VIN conflict is terminal; create a separate session".into(),
                ));
            }
            self.flush_inner()?;
            let time = self.now()?;
            let _ = self.source.disconnect(self.config.operation_timeout);
            self.connected = false;
            self.db()?.disconnected(time)?;
            self.db()?
                .event(time, "reconnect", "caller requested connection recovery")?;
            self.connect_and_vin()?;
            self.discover_selected()?;
            self.state = RecordingState::Recording;
            {
                let time = self.now()?;
                self.db()?.state("recording", time)?;
            };
            Ok(self.progress.clone())
        })();
        self.complete(result, complete);
    }
    /// Change the period for a selected signal; the change is recorded, not retroactive.
    pub fn set_period(
        &mut self,
        index: usize,
        period: Duration,
        complete: impl FnOnce(DataResult<()>),
    ) {
        let result = (|| {
            self.require_recording()?;
            let us = micros(period)?;
            if index >= self.config.signals.len() {
                return Err(invalid("signal index out of range"));
            }
            self.flush_inner()?;
            let time = self.now()?;
            self.db()?.change_period(index, us, time)?;
            self.config.signals[index].period = period;
            self.next_signals[index] = time.elapsed_us;
            Ok(())
        })();
        self.complete(result, complete);
    }
    pub fn finish(&mut self, complete: impl FnOnce(DataResult<RecordingSummary>)) {
        let result = self.finish_inner();
        self.complete(result, complete);
    }
    /// Run until configured duration expires or progress returns Stop, then finalize.
    /// With no duration, the caller must eventually return Stop. Callbacks run after
    /// each scheduled operation and at most 100 ms apart while waiting for due work.
    pub fn run(
        &mut self,
        mut progress: impl FnMut(&RecordingProgress) -> RecordControl,
        complete: impl FnOnce(DataResult<RecordingSummary>),
    ) {
        let result = (|| {
            self.require_recording()?;
            loop {
                let time = self.now()?;
                let end = self
                    .config
                    .duration
                    .map(micros)
                    .transpose()?
                    .map(|d| self.active_start.saturating_add(d));
                if end.is_some_and(|d| time.elapsed_us >= d)
                    || progress(&self.progress) == RecordControl::Stop
                {
                    break;
                }
                self.flush_if_due(time)?;
                let signal = self
                    .next_signals
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, due)| *due)
                    .map(|(i, d)| (i, *d));
                let scan_due = self.config.dtcs.as_ref().map(|_| self.next_scan);
                let next = signal.map(|(_, d)| d).into_iter().chain(scan_due).min();
                if let Some(due) = next.filter(|d| *d <= time.elapsed_us) {
                    if scan_due == Some(due) {
                        let budget = end.map(|e| {
                            Duration::from_micros(e.saturating_sub(time.elapsed_us).max(1) as u64)
                        });
                        self.scan(ScanTrigger::Periodic, budget)?;
                        let after = self.now()?;
                        let period = micros(self.config.dtcs.as_ref().unwrap().period)?;
                        self.next_scan = self.advance_due(due, period, after, "dtc")?;
                    } else if let Some((index, _)) = signal {
                        let selected = self.config.signals[index].clone();
                        let timeout = end
                            .map(|e| {
                                Duration::from_micros(
                                    e.saturating_sub(time.elapsed_us).max(1) as u64
                                )
                            })
                            .map_or(self.config.operation_timeout, |d| {
                                d.min(self.config.operation_timeout)
                            });
                        self.capture(
                            ReadRequest::Signal(selected.signal),
                            selected.target,
                            None,
                            due,
                            timeout,
                        )?;
                        let after = self.now()?;
                        self.next_signals[index] = self.advance_due(
                            due,
                            micros(selected.period)?,
                            after,
                            &selected.signal.id(),
                        )?;
                    }
                } else {
                    self.flush_inner()?;
                    let wait = next.into_iter().chain(end).min().map_or(100_000, |d| {
                        d.saturating_sub(time.elapsed_us).clamp(1, 100_000)
                    });
                    self.clock.sleep(Duration::from_micros(wait as u64));
                }
                if !self.connected {
                    return Err(DataError::State(
                        "connection lost; call reconnect or finish explicitly".into(),
                    ));
                }
            }
            self.finish_inner()
        })();
        self.complete(result, complete);
    }
    fn start_inner(&mut self, path: &Path) -> DataResult<RecordingProgress> {
        if self.state != RecordingState::New {
            return Err(DataError::State("start requires a new recorder".into()));
        }
        let time = self.now()?;
        self.store = Some(Store::create(path, &self.config, time)?);
        self.state = RecordingState::Preparing;
        if let Err(error) = self.connect_and_vin() {
            let time = self.now()?;
            self.db()?.state("start_failed", time)?;
            self.db()?.db()?.execute(
                "UPDATE vehicle SET vin_status='failed' WHERE vin_status='pending'",
                [],
            )?;
            self.db()?
                .event(time, "preparation_failed", &error.to_string())?;
            let _ = self.source.disconnect(self.config.operation_timeout);
            self.connected = false;
            self.db()?.disconnected(time)?;
            return Err(error);
        }
        self.discover_selected()?;
        self.flush_inner()?;
        let time = self.now()?;
        self.active_start = time.elapsed_us;
        self.next_signals = vec![time.elapsed_us; self.config.signals.len()];
        self.next_scan = time.elapsed_us.saturating_add(
            self.config
                .dtcs
                .as_ref()
                .map(|d| micros(d.period))
                .transpose()?
                .unwrap_or(i64::MAX),
        );
        self.db()?.state("recording", time)?;
        self.state = RecordingState::Recording;
        if self.config.dtcs.as_ref().is_some_and(|d| d.beginning) {
            self.scan(ScanTrigger::Beginning, None)?;
        }
        Ok(self.progress.clone())
    }
    fn discover_selected(&mut self) -> DataResult<()> {
        self.db()?
            .db()?
            .execute("UPDATE capture_selection SET capability='unknown'", [])?;
        // Query only support blocks needed by selected PIDs; preserve failures as unknown.
        let mut blocks: Vec<u8> = self
            .config
            .signals
            .iter()
            .filter_map(|s| s.signal.pid())
            .filter(|p| *p != 0)
            .map(|p| ((p - 1) / 32) * 32)
            .collect();
        blocks.sort();
        blocks.dedup();
        for base in blocks {
            let time = self.now()?;
            let observation = self.capture(
                ReadRequest::Supported(base),
                None,
                None,
                time.elapsed_us,
                self.config.operation_timeout,
            )?;
            for (index, selected) in self.config.signals.iter().enumerate() {
                if let Some(pid) = selected.signal.pid() {
                    if pid > base && u16::from(pid) <= u16::from(base) + 32 {
                        let replies: Vec<_> = observation
                            .responses
                            .iter()
                            .filter(|e| selected.target.is_none_or(|s| s == e.sender))
                            .filter_map(|e| {
                                if let ObservationValue::Supported(bits) = e.value {
                                    Some(bits & (1 << (32 - (pid - base))) != 0)
                                } else {
                                    None
                                }
                            })
                            .collect();
                        if !replies.is_empty() {
                            self.db()?.selection_capability(
                                index,
                                if replies.iter().any(|v| *v) {
                                    "supported"
                                } else {
                                    "unsupported"
                                },
                            )?;
                        }
                    }
                }
            }
        }
        self.flush_inner()?;
        Ok(())
    }
    fn connect_and_vin(&mut self) -> DataResult<()> {
        let info = self.source.connect(self.config.operation_timeout)?;
        self.connected = true;
        let time = self.now()?;
        self.db_mut()?.connect(&info, time)?;
        let observation = self.capture(
            ReadRequest::Vin,
            None,
            None,
            time.elapsed_us,
            self.config.operation_timeout,
        )?;
        self.flush_inner()?;
        let vins: Vec<_> = observation
            .responses
            .iter()
            .filter_map(|e| {
                if let ObservationValue::Vin(v) = &e.value {
                    Some(v.as_str())
                } else {
                    None
                }
            })
            .collect();
        if vins.is_empty() {
            return Err(DataError::State(
                "VIN could not be established; preparation/reconnect failed".into(),
            ));
        }
        let existing = self.db()?.vin()?;
        if vins.iter().any(|v| *v != vins[0]) || existing.as_ref().is_some_and(|v| v != vins[0]) {
            self.db()?.vin_conflict()?;
            let time = self.now()?;
            self.db()?.event(
                time,
                "vin_conflict",
                "vehicle identity changed or ECUs disagree",
            )?;
            return Err(DataError::State(
                "conflicting VIN observations; start a separate session".into(),
            ));
        }
        self.db()?.set_vin(vins[0])?;
        Ok(())
    }
    fn capture(
        &mut self,
        request: ReadRequest,
        target: Option<crate::obd::Sender>,
        group: Option<i64>,
        due: i64,
        timeout: Duration,
    ) -> DataResult<Observation> {
        if self.poisoned {
            return Err(DataError::State(
                "storage failed; acquisition stopped".into(),
            ));
        }
        let start = self.now()?;
        if let CommitPolicy::Batch { max_age, .. } = self.config.commit_policy {
            if self.pending.first().is_some_and(|p| {
                start
                    .elapsed_us
                    .saturating_add(micros(timeout).unwrap_or(i64::MAX))
                    >= p.end
                        .elapsed_us
                        .saturating_add(micros(max_age).unwrap_or(0))
            }) {
                self.flush_inner()?;
            }
        }
        let observation = if self.connected {
            self.source.read(&request, timeout)
        } else {
            Observation {
                error: Some("connection unavailable".into()),
                ..Observation::default()
            }
        };
        self.save(request, target, group, due, start, observation)
    }
    fn save(
        &mut self,
        request: ReadRequest,
        target: Option<crate::obd::Sender>,
        group: Option<i64>,
        due: i64,
        start: Timestamp,
        mut observation: Observation,
    ) -> DataResult<Observation> {
        let end = self.now()?;
        let mut truncated = false;
        if observation.bytes() > self.config.max_observation_bytes {
            truncated = true;
            observation.responses.clear();
            if let Some(raw) = &mut observation.raw_response {
                raw.truncate(self.config.max_observation_bytes / 2);
            }
            observation.error=Some("observation exceeded configured byte limit; decoded rows rejected and evidence truncated".into());
            observation.partial = true;
            observation.recovery = None;
            observation.context = None;
            if let Some(request) = &mut observation.attempted_request {
                request.truncate(256);
            }
        }
        let mut senders = std::collections::BTreeSet::new();
        for response in &mut observation.responses {
            if !senders.insert(response.sender) {
                return Err(invalid("source returned duplicate sender observations"));
            }
            let valid = match (&request, &response.value) {
                (_, ObservationValue::Failure { .. }) => true,
                (ReadRequest::Vin, ObservationValue::Vin(v)) => {
                    v.len() == 17
                        && v.bytes().all(|b| {
                            b.is_ascii_uppercase() && !matches!(b, b'I' | b'O' | b'Q')
                                || b.is_ascii_digit()
                        })
                }
                (
                    ReadRequest::Signal(Signal::Pid(_) | Signal::AdapterVoltage),
                    ObservationValue::Number(v),
                ) => v.is_finite(),
                (
                    ReadRequest::Signal(Signal::RawPid(_)) | ReadRequest::Diagnostic(_),
                    ObservationValue::Raw(_),
                ) => true,
                (ReadRequest::Dtcs(_), ObservationValue::Dtcs(_))
                | (ReadRequest::Supported(_), ObservationValue::Supported(_)) => true,
                _ => false,
            };
            if !valid {
                response.value = ObservationValue::Failure {
                    kind: "invalid_source_value".into(),
                    message: "source value does not match requested data type".into(),
                };
            }
        }
        if target.is_some_and(|s| !senders.contains(&s)) {
            observation.error = Some("selected ECU did not respond".into());
        }
        if observation.responses.is_empty() && observation.error.is_none() {
            observation.error = Some("no ECU observations returned".into());
        }
        let bytes = observation.bytes();
        if let CommitPolicy::Batch {
            max_acquisitions,
            max_bytes,
            ..
        } = self.config.commit_policy
        {
            if self.pending.len() >= max_acquisitions
                || self.pending_bytes.saturating_add(bytes) > max_bytes
            {
                self.flush_inner()?;
            }
        }
        self.sequence = self
            .sequence
            .checked_add(1)
            .ok_or_else(|| invalid("acquisition sequence exhausted"))?;
        self.progress.acquired += 1;
        if observation.failed() {
            self.progress.failed_acquisitions += 1;
        }
        self.pending_bytes = self.pending_bytes.saturating_add(bytes);
        self.pending.push(Pending {
            sequence: self.sequence,
            request,
            due,
            start,
            end,
            observation: observation.clone(),
            scan_group: group,
            target,
            truncated,
        });
        if matches!(self.config.commit_policy, CommitPolicy::EachAcquisition) {
            self.flush_inner()?;
        } else {
            self.flush_if_due(end)?;
        }
        if observation.disconnected {
            self.flush_inner()?;
            self.connected = false;
            self.db()?.disconnected(end)?;
            self.db()?
                .event(end, "disconnected", "source requires reconnection")?;
        }
        Ok(observation)
    }
    fn scan(&mut self, trigger: ScanTrigger, budget: Option<Duration>) -> DataResult<bool> {
        self.flush_inner()?;
        let time = self.now()?;
        let mut requests: Vec<ReadRequest> = self
            .config
            .dtcs
            .as_ref()
            .map(|d| d.kinds.iter().copied().map(ReadRequest::Dtcs).collect())
            .unwrap_or_default();
        requests.extend(
            self.config
                .profiles
                .iter()
                .cloned()
                .map(ReadRequest::Diagnostic),
        );
        if requests.is_empty() {
            return Ok(true);
        }
        let group = self
            .db()?
            .begin_scan(trigger, time, &format!("{requests:?}"))?;
        let budget = budget
            .or_else(|| (trigger == ScanTrigger::Ending).then_some(self.config.shutdown_timeout));
        let deadline = budget
            .map(micros)
            .transpose()?
            .map(|b| time.elapsed_us.saturating_add(b));
        let mut complete = true;
        for request in requests {
            let target = if let ReadRequest::Diagnostic(p) = &request {
                p.target
            } else {
                None
            };
            let now = self.now()?;
            let result = if deadline.is_some_and(|end| now.elapsed_us >= end) {
                self.save(
                    request,
                    target,
                    Some(group),
                    time.elapsed_us,
                    now,
                    Observation {
                        error: Some("scan budget expired before request".into()),
                        ..Observation::default()
                    },
                )?
            } else {
                let timeout = deadline
                    .map(|end| {
                        Duration::from_micros(end.saturating_sub(now.elapsed_us).max(1) as u64)
                    })
                    .map_or(self.config.operation_timeout, |d| {
                        d.min(self.config.operation_timeout)
                    });
                self.capture(request, target, Some(group), time.elapsed_us, timeout)?
            };
            complete &= !result.failed();
        }
        self.flush_inner()?;
        let time = self.now()?;
        self.db()?.end_scan(group, time, complete)?;
        Ok(complete)
    }
    fn finish_inner(&mut self) -> DataResult<RecordingSummary> {
        if !matches!(
            self.state,
            RecordingState::Recording | RecordingState::Failed
        ) || self.poisoned
        {
            return Err(DataError::State("recording cannot be finalized".into()));
        }
        let valid: bool =
            self.db()?
                .db()?
                .query_row("SELECT vin_status='verified' FROM vehicle", [], |r| {
                    r.get(0)
                })?;
        if !valid {
            return Err(DataError::State(
                "cannot finalize or scan a session without a verified, non-conflicting VIN".into(),
            ));
        }
        let ending = if self.config.dtcs.as_ref().is_some_and(|d| d.ending) {
            self.scan(ScanTrigger::Ending, None)?
        } else {
            false
        };
        self.flush_inner()?;
        let disconnected = self.source.disconnect(self.config.operation_timeout);
        self.connected = false;
        let time = self.now()?;
        self.db()?.disconnected(time)?;
        if let Err(e) = disconnected {
            self.db()?.event(time, "disconnect_error", &e.to_string())?;
        }
        self.db_mut()?.finalize(time, ending)?;
        self.state = RecordingState::Finished;
        let store = self.db()?;
        Ok(RecordingSummary {
            session_id: store.id.clone(),
            path: store.path.clone(),
            progress: self.progress.clone(),
            ending_scan_complete: ending,
        })
    }
    fn flush_if_due(&mut self, time: Timestamp) -> DataResult<()> {
        if let CommitPolicy::Batch {
            max_age,
            max_acquisitions,
            max_bytes,
        } = self.config.commit_policy
        {
            if self.pending.len() >= max_acquisitions
                || self.pending_bytes >= max_bytes
                || self.pending.first().is_some_and(|p| {
                    time.elapsed_us.saturating_sub(p.end.elapsed_us) >= micros(max_age).unwrap_or(0)
                })
            {
                self.flush_inner()?;
            }
        }
        Ok(())
    }
    fn flush_inner(&mut self) -> DataResult<()> {
        if self.poisoned {
            return Err(DataError::State(
                "storage failed; pending data is not automatically retried".into(),
            ));
        }
        if self.pending.is_empty() {
            return Ok(());
        }
        if let Err(e) = self.db()?.flush(&self.pending) {
            self.poisoned = true;
            self.state = RecordingState::Failed;
            return Err(e);
        }
        self.progress.committed = self.progress.acquired;
        self.pending.clear();
        self.pending_bytes = 0;
        Ok(())
    }
    fn advance_due(&self, due: i64, period: i64, now: Timestamp, label: &str) -> DataResult<i64> {
        let intervals = now.elapsed_us.saturating_sub(due).max(0) / period + 1;
        if intervals > 1 {
            self.db()?.event(
                now,
                "missed_schedule",
                &format!("{label}: {} skipped due times", intervals - 1),
            )?;
        }
        Ok(due.saturating_add(intervals.saturating_mul(period)))
    }
    fn now(&mut self) -> DataResult<Timestamp> {
        let now = self.clock.now();
        if now.elapsed_us < 0 {
            return Err(invalid("negative monotonic elapsed time"));
        }
        if let Some(previous) = self.last_time {
            if now.elapsed_us < previous.elapsed_us {
                return Err(invalid("monotonic clock moved backwards"));
            }
            let difference = (i128::from(now.utc_us) - i128::from(previous.utc_us))
                - (i128::from(now.elapsed_us) - i128::from(previous.elapsed_us));
            if difference.abs() > 1_000_000 {
                if let Some(store) = &self.store {
                    store.event(
                        now,
                        "clock_adjustment",
                        &format!("UTC/monotonic delta differed by {difference} us"),
                    )?;
                }
            }
        }
        self.last_time = Some(now);
        Ok(now)
    }
    fn db(&self) -> DataResult<&Store> {
        self.store
            .as_ref()
            .ok_or_else(|| DataError::State("start recording first".into()))
    }
    fn db_mut(&mut self) -> DataResult<&mut Store> {
        self.store
            .as_mut()
            .ok_or_else(|| DataError::State("start recording first".into()))
    }
    fn require_recording(&self) -> DataResult<()> {
        if self.state != RecordingState::Recording || !self.connected || self.poisoned {
            return Err(DataError::State(
                "recording is not active; reconnect or finish explicitly".into(),
            ));
        }
        Ok(())
    }
    fn complete<T>(&mut self, result: DataResult<T>, callback: impl FnOnce(DataResult<T>)) {
        if let Err(error) = &result {
            if self.store.is_some() && self.state != RecordingState::Finished {
                self.state = RecordingState::Failed;
                if matches!(error, DataError::Io(_) | DataError::Sqlite(_)) {
                    self.poisoned = true;
                }
                if !self.poisoned {
                    if let Some(store) = &self.store {
                        let time = self.clock.now();
                        let _ = store.event(time, "recorder_error", &error.to_string());
                    }
                }
            }
        }
        callback(result);
    }
}
