CREATE TABLE session (
 id TEXT PRIMARY KEY, format_id TEXT NOT NULL, schema_version INTEGER NOT NULL,
 created_utc_us INTEGER NOT NULL, started_utc_us INTEGER, ended_utc_us INTEGER, finalized_utc_us INTEGER,
 exported_utc_us INTEGER, export_cutoff_sequence INTEGER, previous_session TEXT,
 status TEXT NOT NULL CHECK(status IN ('preparing','recording','finalizing','completed','interrupted','failed','start_failed','snapshot')),
 producer TEXT NOT NULL, producer_version TEXT NOT NULL, library_version TEXT NOT NULL, platform TEXT NOT NULL,
 config TEXT NOT NULL, committed_sequence INTEGER NOT NULL DEFAULT 0,
 manufacturer_coverage TEXT NOT NULL, ending_scan_complete INTEGER NOT NULL DEFAULT 0 CHECK(ending_scan_complete IN (0,1))
);
CREATE TABLE vehicle (id INTEGER PRIMARY KEY CHECK(id=1), vin TEXT, vin_status TEXT NOT NULL CHECK(vin_status IN ('pending','verified','conflict','failed')));
CREATE TABLE vehicle_field (name TEXT PRIMARY KEY, value TEXT, source TEXT, source_version TEXT, observed_utc_us INTEGER NOT NULL);
CREATE TABLE connection_segment (id INTEGER PRIMARY KEY, opened_utc_us INTEGER NOT NULL, opened_elapsed_us INTEGER NOT NULL, closed_utc_us INTEGER, transport TEXT NOT NULL, adapter TEXT NOT NULL);
CREATE TABLE ecu (id INTEGER PRIMARY KEY, segment_id INTEGER NOT NULL REFERENCES connection_segment(id), protocol INTEGER NOT NULL, sender TEXT NOT NULL, UNIQUE(segment_id,protocol,sender));
CREATE TABLE signal_definition (id TEXT PRIMARY KEY, service INTEGER, pid INTEGER, unit TEXT NOT NULL, decoder_version TEXT NOT NULL, raw_only INTEGER NOT NULL CHECK(raw_only IN (0,1)));
CREATE TABLE capture_selection (id INTEGER PRIMARY KEY, signal_id TEXT NOT NULL REFERENCES signal_definition(id), target TEXT, period_us INTEGER NOT NULL CHECK(period_us>0), enabled_elapsed_us INTEGER NOT NULL, disabled_elapsed_us INTEGER, capability TEXT NOT NULL);
CREATE TABLE diagnostic_profile (id TEXT PRIMARY KEY, version TEXT NOT NULL, namespace TEXT NOT NULL, request BLOB NOT NULL, target TEXT, header TEXT, receive_filter TEXT);
CREATE TABLE acquisition (
 id INTEGER PRIMARY KEY, segment_id INTEGER REFERENCES connection_segment(id), kind TEXT NOT NULL, request TEXT NOT NULL,
 due_elapsed_us INTEGER NOT NULL, operation_started_utc_us INTEGER NOT NULL, operation_started_elapsed_us INTEGER NOT NULL,
 operation_completed_utc_us INTEGER NOT NULL, operation_completed_elapsed_us INTEGER NOT NULL,
 outcome TEXT NOT NULL CHECK(outcome IN ('ok','partial','failed')), error TEXT, recovery TEXT, target TEXT,
 CHECK(operation_completed_elapsed_us>=operation_started_elapsed_us)
);
CREATE TABLE transaction_evidence (
 acquisition_id INTEGER PRIMARY KEY REFERENCES acquisition(id), scope TEXT NOT NULL, raw_response BLOB, attempted_request BLOB,
 protocol INTEGER, context TEXT NOT NULL, partial INTEGER NOT NULL CHECK(partial IN (0,1)), truncated INTEGER NOT NULL CHECK(truncated IN (0,1))
);
CREATE TABLE vin_observation (id INTEGER PRIMARY KEY, acquisition_id INTEGER NOT NULL REFERENCES acquisition(id), ecu_id INTEGER NOT NULL REFERENCES ecu(id), vin TEXT NOT NULL);
CREATE TABLE measurement (
 id INTEGER PRIMARY KEY, acquisition_id INTEGER NOT NULL REFERENCES acquisition(id), ecu_id INTEGER NOT NULL REFERENCES ecu(id),
 signal_id TEXT NOT NULL REFERENCES signal_definition(id), observed_utc_us INTEGER NOT NULL, observed_elapsed_us INTEGER NOT NULL,
 value_type TEXT NOT NULL CHECK(value_type IN ('number','blob')), number_value REAL, blob_value BLOB, unit TEXT NOT NULL,
 CHECK((value_type='number' AND number_value IS NOT NULL AND blob_value IS NULL) OR (value_type='blob' AND number_value IS NULL AND blob_value IS NOT NULL)),
 UNIQUE(acquisition_id,ecu_id,signal_id)
);
CREATE TABLE capability_observation (acquisition_id INTEGER NOT NULL REFERENCES acquisition(id), ecu_id INTEGER NOT NULL REFERENCES ecu(id), base INTEGER NOT NULL, bitmap INTEGER NOT NULL, PRIMARY KEY(acquisition_id,ecu_id,base));
CREATE TABLE dtc_scan_group (id INTEGER PRIMARY KEY, trigger TEXT NOT NULL CHECK(trigger IN ('beginning','periodic','ending','manual')), started_utc_us INTEGER NOT NULL, started_elapsed_us INTEGER NOT NULL, ended_utc_us INTEGER, ended_elapsed_us INTEGER, requested TEXT NOT NULL, coverage TEXT NOT NULL);
CREATE TABLE dtc_scan_result (
 id INTEGER PRIMARY KEY, group_id INTEGER NOT NULL REFERENCES dtc_scan_group(id), acquisition_id INTEGER NOT NULL REFERENCES acquisition(id), ecu_id INTEGER REFERENCES ecu(id),
 category TEXT NOT NULL, namespace TEXT NOT NULL, profile_id TEXT REFERENCES diagnostic_profile(id), outcome TEXT NOT NULL CHECK(outcome IN ('ok','failed','raw_only')),
 code_count INTEGER CHECK(code_count>=0), raw_payload BLOB, error TEXT,
 CHECK((outcome='ok' AND code_count IS NOT NULL) OR (outcome!='ok' AND code_count IS NULL)),
 UNIQUE(acquisition_id,ecu_id)
);
CREATE TABLE dtc_entry (id INTEGER PRIMARY KEY, scan_id INTEGER NOT NULL REFERENCES dtc_scan_result(id), ordinal INTEGER NOT NULL CHECK(ordinal>=0), code TEXT NOT NULL, code_bytes BLOB NOT NULL, status_bytes BLOB, interpretation TEXT, UNIQUE(scan_id,ordinal));
CREATE TABLE event (id INTEGER PRIMARY KEY, utc_us INTEGER NOT NULL, elapsed_us INTEGER NOT NULL, kind TEXT NOT NULL, detail TEXT NOT NULL);
CREATE INDEX measurement_time ON measurement(observed_elapsed_us,id);
CREATE INDEX measurement_signal_ecu ON measurement(signal_id,ecu_id,observed_elapsed_us,id);
CREATE INDEX acquisition_time ON acquisition(operation_completed_elapsed_us,id);
CREATE INDEX scan_group_time ON dtc_scan_group(started_elapsed_us,id);
CREATE INDEX scan_acquisition ON dtc_scan_result(acquisition_id,id);
CREATE INDEX dtc_history ON dtc_entry(code,scan_id);
CREATE INDEX event_time ON event(elapsed_us,id);
CREATE TABLE recording_policy (id INTEGER PRIMARY KEY CHECK(id=1),duration_us INTEGER,operation_timeout_us INTEGER NOT NULL,shutdown_timeout_us INTEGER NOT NULL,commit_mode TEXT NOT NULL,batch_max_age_us INTEGER,batch_max_acquisitions INTEGER,batch_max_bytes INTEGER,max_observation_bytes INTEGER NOT NULL,dtc_period_us INTEGER,dtc_beginning INTEGER NOT NULL,dtc_ending INTEGER NOT NULL);
CREATE TABLE dtc_selection (service INTEGER PRIMARY KEY CHECK(service IN (3,7,10)));
CREATE TABLE selection_change (id INTEGER PRIMARY KEY, selection_id INTEGER NOT NULL REFERENCES capture_selection(id), utc_us INTEGER NOT NULL, elapsed_us INTEGER NOT NULL, period_us INTEGER NOT NULL CHECK(period_us>0));
