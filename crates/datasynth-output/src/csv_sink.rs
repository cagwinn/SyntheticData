//! CSV output sink with optional disk space monitoring.
//!
//! Uses zero-allocation write-through formatting via `write!()` directly to the
//! buffered writer, avoiding the per-row `format!()` String allocation that was
//! the main bottleneck in CSV output (Phase 3 I/O optimization).

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::sync::Arc;

use datasynth_core::error::{SynthError, SynthResult};
use datasynth_core::models::subledger::ar::{
    DunningItem, DunningLetter, DunningRun, OnAccountPayment, PaymentCorrection, ShortPayment,
};
use datasynth_core::models::JournalEntry;
use datasynth_core::traits::Sink;
use datasynth_core::{DiskSpaceGuard, DiskSpaceGuardConfig};

/// CSV sink for journal entry output with optional disk space monitoring.
pub struct CsvSink {
    writer: BufWriter<File>,
    items_written: u64,
    bytes_written: u64,
    header_written: bool,
    /// Optional disk space guard for monitoring available space
    disk_guard: Option<Arc<DiskSpaceGuard>>,
    /// Interval for disk checks (every N items)
    check_interval: u64,
}

impl CsvSink {
    /// Create a new CSV sink.
    pub fn new(path: PathBuf) -> SynthResult<Self> {
        let file = File::create(&path)?;
        Ok(Self {
            writer: BufWriter::with_capacity(256 * 1024, file),
            items_written: 0,
            bytes_written: 0,
            header_written: false,
            disk_guard: None,
            check_interval: 500,
        })
    }

    /// Create a new CSV sink with disk space monitoring.
    pub fn with_disk_guard(path: PathBuf, min_free_mb: usize) -> SynthResult<Self> {
        let file = File::create(&path)?;
        let disk_config = DiskSpaceGuardConfig::with_min_free_mb(min_free_mb).with_path(&path);
        let disk_guard = Arc::new(DiskSpaceGuard::new(disk_config));

        Ok(Self {
            writer: BufWriter::with_capacity(256 * 1024, file),
            items_written: 0,
            bytes_written: 0,
            header_written: false,
            disk_guard: Some(disk_guard),
            check_interval: 500,
        })
    }

    /// Set a custom disk guard.
    pub fn set_disk_guard(&mut self, guard: Arc<DiskSpaceGuard>) {
        self.disk_guard = Some(guard);
    }

    /// Set the disk check interval.
    pub fn set_check_interval(&mut self, interval: u64) {
        self.check_interval = interval;
    }

    /// Check disk space if guard is configured.
    fn check_disk_space(&self) -> SynthResult<()> {
        if let Some(guard) = &self.disk_guard {
            if self.items_written.is_multiple_of(self.check_interval) {
                guard
                    .check()
                    .map_err(|e| SynthError::disk_exhausted(e.available_mb, e.required_mb))?;
            }
        }
        Ok(())
    }

    /// Record bytes written for tracking.
    fn record_write(&self, bytes: u64) {
        if let Some(guard) = &self.disk_guard {
            guard.record_write(bytes);
        }
    }

    fn write_header(&mut self) -> SynthResult<()> {
        if self.header_written {
            return Ok(());
        }

        let header = "document_id,company_code,fiscal_year,fiscal_period,posting_date,\
            document_type,currency,source,line_number,gl_account,debit_amount,credit_amount,\
            trading_partner,fraud_type,anomaly_type\n";
        let bytes = header.as_bytes();
        self.writer.write_all(bytes)?;
        self.bytes_written += bytes.len() as u64;
        self.record_write(bytes.len() as u64);
        self.header_written = true;
        Ok(())
    }

    /// Get total bytes written.
    pub fn bytes_written(&self) -> u64 {
        self.bytes_written
    }
}

impl Sink for CsvSink {
    type Item = JournalEntry;

    fn write(&mut self, item: Self::Item) -> SynthResult<()> {
        // Check disk space periodically
        self.check_disk_space()?;

        self.write_header()?;

        // Write directly to BufWriter using write!() — no intermediate String allocation.
        // This is the Phase 3 optimization: format!() allocated a new String per row,
        // while write!() formats directly into the BufWriter's internal buffer.
        // SP3.6 — emit canonical SAP source code when priors are loaded;
        // fall back to the TransactionSource debug label otherwise.
        let source_label: std::borrow::Cow<str> = match &item.header.sap_source_code {
            Some(code) => std::borrow::Cow::Borrowed(code.as_str()),
            None => std::borrow::Cow::Owned(format!("{:?}", item.header.source)),
        };
        // v5.17.0 — fraud_type and anomaly_type category columns (schema parity
        // with output_writer::write_journal_entries_csv).
        let fraud_type_str = item
            .header
            .fraud_type
            .map(|ft| format!("{ft:?}"))
            .unwrap_or_default();
        let anomaly_type_str = item
            .header
            .anomaly_type
            .as_deref()
            .unwrap_or("")
            .to_string();
        for line in &item.lines {
            let bytes_before = self.bytes_written;
            writeln!(
                self.writer,
                "{},{},{},{},{},{},{},{},{},{},{},{},{},{},{}",
                item.header.document_id,
                item.header.company_code,
                item.header.fiscal_year,
                item.header.fiscal_period,
                item.header.posting_date,
                item.header.document_type,
                item.header.currency,
                source_label,
                line.line_number,
                line.gl_account,
                line.debit_amount,
                line.credit_amount,
                line.trading_partner.as_deref().unwrap_or(""),
                fraud_type_str,
                anomaly_type_str,
            )?;
            // Estimate bytes written (exact tracking would require a counting writer)
            let estimated_bytes = 100u64; // conservative estimate per line
            self.bytes_written += estimated_bytes;
            self.record_write(self.bytes_written - bytes_before);
        }

        self.items_written += 1;
        Ok(())
    }

    fn flush(&mut self) -> SynthResult<()> {
        self.writer.flush()?;
        Ok(())
    }

    fn close(mut self) -> SynthResult<()> {
        self.flush()?;
        Ok(())
    }

    fn items_written(&self) -> u64 {
        self.items_written
    }
}

// ============================================================================
// Dunning Run CSV Sink
// ============================================================================

/// CSV sink for dunning runs.
pub struct DunningRunCsvSink {
    writer: BufWriter<File>,
    items_written: u64,
    header_written: bool,
}

impl DunningRunCsvSink {
    /// Create a new dunning run CSV sink.
    pub fn new(path: PathBuf) -> SynthResult<Self> {
        let file = File::create(&path)?;
        Ok(Self {
            writer: BufWriter::with_capacity(256 * 1024, file),
            items_written: 0,
            header_written: false,
        })
    }

    fn write_header(&mut self) -> SynthResult<()> {
        if self.header_written {
            return Ok(());
        }

        let header = "run_id,company_code,run_date,dunning_date,customers_evaluated,\
            customers_with_letters,letters_generated,total_amount_dunned,\
            total_dunning_charges,total_interest_amount,status,started_at,completed_at\n";
        self.writer.write_all(header.as_bytes())?;
        self.header_written = true;
        Ok(())
    }
}

impl Sink for DunningRunCsvSink {
    type Item = DunningRun;

    fn write(&mut self, item: Self::Item) -> SynthResult<()> {
        self.write_header()?;

        writeln!(
            self.writer,
            "{},{},{},{},{},{},{},{},{},{},{:?},{},{}",
            item.run_id,
            item.company_code,
            item.run_date,
            item.dunning_date,
            item.customers_evaluated,
            item.customers_with_letters,
            item.letters_generated,
            item.total_amount_dunned,
            item.total_dunning_charges,
            item.total_interest_amount,
            item.status,
            item.started_at,
            item.completed_at.map(|d| d.to_string()).unwrap_or_default(),
        )?;

        self.items_written += 1;
        Ok(())
    }

    fn flush(&mut self) -> SynthResult<()> {
        self.writer.flush()?;
        Ok(())
    }

    fn close(mut self) -> SynthResult<()> {
        self.flush()?;
        Ok(())
    }

    fn items_written(&self) -> u64 {
        self.items_written
    }
}

// ============================================================================
// Dunning Letter CSV Sink
// ============================================================================

/// CSV sink for dunning letters.
pub struct DunningLetterCsvSink {
    writer: BufWriter<File>,
    items_written: u64,
    header_written: bool,
}

impl DunningLetterCsvSink {
    /// Create a new dunning letter CSV sink.
    pub fn new(path: PathBuf) -> SynthResult<Self> {
        let file = File::create(&path)?;
        Ok(Self {
            writer: BufWriter::with_capacity(256 * 1024, file),
            items_written: 0,
            header_written: false,
        })
    }

    fn write_header(&mut self) -> SynthResult<()> {
        if self.header_written {
            return Ok(());
        }

        let header = "letter_id,dunning_run_id,company_code,customer_id,customer_name,\
            dunning_level,dunning_date,total_dunned_amount,dunning_charges,\
            interest_amount,total_amount_due,currency,payment_deadline,\
            is_sent,sent_date,response_type,status\n";
        self.writer.write_all(header.as_bytes())?;
        self.header_written = true;
        Ok(())
    }
}

impl Sink for DunningLetterCsvSink {
    type Item = DunningLetter;

    fn write(&mut self, item: Self::Item) -> SynthResult<()> {
        self.write_header()?;

        writeln!(
            self.writer,
            "{},{},{},{},\"{}\",{},{},{},{},{},{},{},{},{},{},{:?},{:?}",
            item.letter_id,
            item.dunning_run_id,
            item.company_code,
            item.customer_id,
            item.customer_name.replace('"', "\"\""),
            item.dunning_level,
            item.dunning_date,
            item.total_dunned_amount,
            item.dunning_charges,
            item.interest_amount,
            item.total_amount_due,
            item.currency,
            item.payment_deadline,
            item.is_sent,
            item.sent_date.map(|d| d.to_string()).unwrap_or_default(),
            item.response_type,
            item.status,
        )?;

        self.items_written += 1;
        Ok(())
    }

    fn flush(&mut self) -> SynthResult<()> {
        self.writer.flush()?;
        Ok(())
    }

    fn close(mut self) -> SynthResult<()> {
        self.flush()?;
        Ok(())
    }

    fn items_written(&self) -> u64 {
        self.items_written
    }
}

// ============================================================================
// Dunning Item CSV Sink
// ============================================================================

/// CSV sink for dunning items.
pub struct DunningItemCsvSink {
    writer: BufWriter<File>,
    items_written: u64,
    header_written: bool,
}

impl DunningItemCsvSink {
    /// Create a new dunning item CSV sink.
    pub fn new(path: PathBuf) -> SynthResult<Self> {
        let file = File::create(&path)?;
        Ok(Self {
            writer: BufWriter::with_capacity(256 * 1024, file),
            items_written: 0,
            header_written: false,
        })
    }

    fn write_header(&mut self) -> SynthResult<()> {
        if self.header_written {
            return Ok(());
        }

        let header = "letter_id,invoice_number,invoice_date,due_date,original_amount,\
            open_amount,days_overdue,interest_amount,previous_dunning_level,\
            new_dunning_level,is_blocked,block_reason\n";
        self.writer.write_all(header.as_bytes())?;
        self.header_written = true;
        Ok(())
    }

    /// Write a dunning item with its associated letter ID.
    pub fn write_with_letter_id(&mut self, letter_id: &str, item: &DunningItem) -> SynthResult<()> {
        self.write_header()?;

        writeln!(
            self.writer,
            "{},{},{},{},{},{},{},{},{},{},{},{}",
            letter_id,
            item.invoice_number,
            item.invoice_date,
            item.due_date,
            item.original_amount,
            item.open_amount,
            item.days_overdue,
            item.interest_amount,
            item.previous_dunning_level,
            item.new_dunning_level,
            item.is_blocked,
            item.block_reason.as_deref().unwrap_or(""),
        )?;

        self.items_written += 1;
        Ok(())
    }

    /// Flush the writer.
    pub fn flush(&mut self) -> SynthResult<()> {
        self.writer.flush()?;
        Ok(())
    }
}

// ============================================================================
// Payment Correction CSV Sink
// ============================================================================

/// CSV sink for payment corrections.
pub struct PaymentCorrectionCsvSink {
    writer: BufWriter<File>,
    items_written: u64,
    header_written: bool,
}

impl PaymentCorrectionCsvSink {
    /// Create a new payment correction CSV sink.
    pub fn new(path: PathBuf) -> SynthResult<Self> {
        let file = File::create(&path)?;
        Ok(Self {
            writer: BufWriter::with_capacity(256 * 1024, file),
            items_written: 0,
            header_written: false,
        })
    }

    fn write_header(&mut self) -> SynthResult<()> {
        if self.header_written {
            return Ok(());
        }

        let header = "correction_id,company_code,customer_id,original_payment_id,\
            correction_type,original_amount,correction_amount,currency,\
            correction_date,reversal_je_id,correcting_payment_id,status,\
            reason,bank_reference,chargeback_code,fee_amount\n";
        self.writer.write_all(header.as_bytes())?;
        self.header_written = true;
        Ok(())
    }
}

impl Sink for PaymentCorrectionCsvSink {
    type Item = PaymentCorrection;

    fn write(&mut self, item: Self::Item) -> SynthResult<()> {
        self.write_header()?;

        writeln!(
            self.writer,
            "{},{},{},{},{:?},{},{},{},{},{},{},{:?},\"{}\",{},{},{}",
            item.correction_id,
            item.company_code,
            item.customer_id,
            item.original_payment_id,
            item.correction_type,
            item.original_amount,
            item.correction_amount,
            item.currency,
            item.correction_date,
            item.reversal_je_id.as_deref().unwrap_or(""),
            item.correcting_payment_id.as_deref().unwrap_or(""),
            item.status,
            item.reason.as_deref().unwrap_or("").replace('"', "\"\""),
            item.bank_reference.as_deref().unwrap_or(""),
            item.chargeback_code.as_deref().unwrap_or(""),
            item.fee_amount,
        )?;

        self.items_written += 1;
        Ok(())
    }

    fn flush(&mut self) -> SynthResult<()> {
        self.writer.flush()?;
        Ok(())
    }

    fn close(mut self) -> SynthResult<()> {
        self.flush()?;
        Ok(())
    }

    fn items_written(&self) -> u64 {
        self.items_written
    }
}

// ============================================================================
// Short Payment CSV Sink
// ============================================================================

/// CSV sink for short payments.
pub struct ShortPaymentCsvSink {
    writer: BufWriter<File>,
    items_written: u64,
    header_written: bool,
}

impl ShortPaymentCsvSink {
    /// Create a new short payment CSV sink.
    pub fn new(path: PathBuf) -> SynthResult<Self> {
        let file = File::create(&path)?;
        Ok(Self {
            writer: BufWriter::with_capacity(256 * 1024, file),
            items_written: 0,
            header_written: false,
        })
    }

    fn write_header(&mut self) -> SynthResult<()> {
        if self.header_written {
            return Ok(());
        }

        let header = "short_payment_id,company_code,customer_id,payment_id,invoice_id,\
            expected_amount,paid_amount,short_amount,currency,payment_date,\
            reason_code,reason_description,disposition,credit_memo_id,\
            write_off_je_id,rebill_invoice_id\n";
        self.writer.write_all(header.as_bytes())?;
        self.header_written = true;
        Ok(())
    }
}

impl Sink for ShortPaymentCsvSink {
    type Item = ShortPayment;

    fn write(&mut self, item: Self::Item) -> SynthResult<()> {
        self.write_header()?;

        writeln!(
            self.writer,
            "{},{},{},{},{},{},{},{},{},{},{:?},\"{}\",{:?},{},{},{}",
            item.short_payment_id,
            item.company_code,
            item.customer_id,
            item.payment_id,
            item.invoice_id,
            item.expected_amount,
            item.paid_amount,
            item.short_amount,
            item.currency,
            item.payment_date,
            item.reason_code,
            item.reason_description
                .as_deref()
                .unwrap_or("")
                .replace('"', "\"\""),
            item.disposition,
            item.credit_memo_id.as_deref().unwrap_or(""),
            item.write_off_je_id.as_deref().unwrap_or(""),
            item.rebill_invoice_id.as_deref().unwrap_or(""),
        )?;

        self.items_written += 1;
        Ok(())
    }

    fn flush(&mut self) -> SynthResult<()> {
        self.writer.flush()?;
        Ok(())
    }

    fn close(mut self) -> SynthResult<()> {
        self.flush()?;
        Ok(())
    }

    fn items_written(&self) -> u64 {
        self.items_written
    }
}

// ============================================================================
// On-Account Payment CSV Sink
// ============================================================================

/// CSV sink for on-account payments.
pub struct OnAccountPaymentCsvSink {
    writer: BufWriter<File>,
    items_written: u64,
    header_written: bool,
}

impl OnAccountPaymentCsvSink {
    /// Create a new on-account payment CSV sink.
    pub fn new(path: PathBuf) -> SynthResult<Self> {
        let file = File::create(&path)?;
        Ok(Self {
            writer: BufWriter::with_capacity(256 * 1024, file),
            items_written: 0,
            header_written: false,
        })
    }

    fn write_header(&mut self) -> SynthResult<()> {
        if self.header_written {
            return Ok(());
        }

        let header = "on_account_id,company_code,customer_id,payment_id,amount,\
            remaining_amount,currency,received_date,status,reason,\
            applications_count,notes\n";
        self.writer.write_all(header.as_bytes())?;
        self.header_written = true;
        Ok(())
    }
}

impl Sink for OnAccountPaymentCsvSink {
    type Item = OnAccountPayment;

    fn write(&mut self, item: Self::Item) -> SynthResult<()> {
        self.write_header()?;

        writeln!(
            self.writer,
            "{},{},{},{},{},{},{},{},{:?},{:?},{},\"{}\"",
            item.on_account_id,
            item.company_code,
            item.customer_id,
            item.payment_id,
            item.amount,
            item.remaining_amount,
            item.currency,
            item.received_date,
            item.status,
            item.reason,
            item.applications.len(),
            item.notes.as_deref().unwrap_or("").replace('"', "\"\""),
        )?;

        self.items_written += 1;
        Ok(())
    }

    fn flush(&mut self) -> SynthResult<()> {
        self.writer.flush()?;
        Ok(())
    }

    fn close(mut self) -> SynthResult<()> {
        self.flush()?;
        Ok(())
    }

    fn items_written(&self) -> u64 {
        self.items_written
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use datasynth_core::models::{JournalEntry, JournalEntryHeader, JournalEntryLine};
    use datasynth_core::traits::Sink;
    use rust_decimal_macros::dec;

    /// Build a minimal two-line JournalEntry: line 1 has trading_partner=Some("VENDOR_A"),
    /// line 2 has trading_partner=None.
    fn make_je_with_trading_partner() -> JournalEntry {
        use chrono::NaiveDate;
        let posting_date = NaiveDate::from_ymd_opt(2024, 1, 15).unwrap();
        let header = JournalEntryHeader::new("US10".to_string(), posting_date);
        let mut je = JournalEntry::new(header);

        let mut line1 =
            JournalEntryLine::debit(je.header.document_id, 1, "1000".to_string(), dec!(500.00));
        line1.trading_partner = Some("VENDOR_A".to_string());

        let line2 =
            JournalEntryLine::credit(je.header.document_id, 2, "2000".to_string(), dec!(500.00));
        // line2.trading_partner stays None

        je.lines.push(line1);
        je.lines.push(line2);
        je
    }

    /// Build a JE with fraud_type = DuplicatePayment for category-column tests.
    fn make_je_with_fraud_type() -> JournalEntry {
        use chrono::NaiveDate;
        use datasynth_core::models::FraudType;
        let posting_date = NaiveDate::from_ymd_opt(2024, 6, 1).unwrap();
        let mut header = JournalEntryHeader::new("FR10".to_string(), posting_date);
        header.is_fraud = true;
        header.fraud_type = Some(FraudType::DuplicatePayment);
        let mut je = JournalEntry::new(header);
        let line1 =
            JournalEntryLine::debit(je.header.document_id, 1, "2000".to_string(), dec!(200.00));
        let line2 =
            JournalEntryLine::credit(je.header.document_id, 2, "1000".to_string(), dec!(200.00));
        je.lines.push(line1);
        je.lines.push(line2);
        je
    }

    #[test]
    fn csv_writer_includes_trading_partner_column() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let path = tmp.path().to_path_buf();

        let mut sink = super::CsvSink::new(path.clone()).unwrap();
        sink.write(make_je_with_trading_partner()).unwrap();
        sink.flush().unwrap();

        let contents = std::fs::read_to_string(&path).unwrap();
        let mut lines = contents.lines();

        // Header must include trading_partner column.
        let header = lines.next().expect("no header line");
        assert!(
            header.contains("trading_partner"),
            "header missing trading_partner column; got: {header}"
        );
        // v5.17.0 — header must also include fraud_type and anomaly_type.
        assert!(
            header.contains("fraud_type"),
            "header missing fraud_type column; got: {header}"
        );
        assert!(
            header.contains("anomaly_type"),
            "header missing anomaly_type column; got: {header}"
        );

        // First data row (debit line): trading_partner=VENDOR_A, fraud_type=empty, anomaly_type=empty.
        let row1 = lines.next().expect("no first data row");
        assert!(
            row1.contains(",VENDOR_A,"),
            "expected row1 to contain ',VENDOR_A,'; got: {row1}"
        );
        // Row ends with ",,": trading_partner=VENDOR_A, fraud_type=empty, anomaly_type=empty.
        assert!(
            row1.ends_with("VENDOR_A,,"),
            "expected row1 to end with 'VENDOR_A,,'; got: {row1}"
        );

        // Second data row (credit line): trading_partner=empty, both category cols empty → ends ",,".
        let row2 = lines.next().expect("no second data row");
        assert!(
            row2.ends_with(",,"),
            "expected row2 to end with ',,' (empty trading_partner, fraud_type, anomaly_type); got: {row2}"
        );
    }

    /// v5.17.0 — fraud_type column in CsvSink streaming writer emits Debug variant name.
    #[test]
    fn csv_sink_fraud_type_column_populated() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let path = tmp.path().to_path_buf();

        let mut sink = super::CsvSink::new(path.clone()).unwrap();
        sink.write(make_je_with_fraud_type()).unwrap();
        sink.flush().unwrap();

        let contents = std::fs::read_to_string(&path).unwrap();
        let mut lines = contents.lines();
        let _header = lines.next().expect("no header");
        let row1 = lines.next().expect("no data row");

        // fraud_type column must contain "DuplicatePayment".
        assert!(
            row1.contains("DuplicatePayment"),
            "expected 'DuplicatePayment' in fraud_type column; got: {row1}"
        );
    }

    /// v5.17.0 — fraud_type and anomaly_type columns are empty strings when None.
    #[test]
    fn csv_sink_fraud_type_none_is_empty() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let path = tmp.path().to_path_buf();

        let mut sink = super::CsvSink::new(path.clone()).unwrap();
        sink.write(make_je_with_trading_partner()).unwrap();
        sink.flush().unwrap();

        let contents = std::fs::read_to_string(&path).unwrap();
        let mut lines = contents.lines();
        let _header = lines.next().expect("no header");
        let row1 = lines.next().expect("no data row");

        // Row has 15 columns; cols 14 and 15 (fraud_type, anomaly_type) must be empty.
        let cols: Vec<&str> = row1.split(',').collect();
        assert_eq!(
            cols.len(),
            15,
            "expected 15 columns in CsvSink row, got {}; row: {row1}",
            cols.len()
        );
        assert!(
            cols[13].is_empty(),
            "expected empty fraud_type (col 14); got: {}",
            cols[13]
        );
        assert!(
            cols[14].is_empty(),
            "expected empty anomaly_type (col 15); got: {}",
            cols[14]
        );
    }
}
