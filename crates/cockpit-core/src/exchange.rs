use std::io::Write;

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use csv::{Terminator, Writer, WriterBuilder};
use rust_xlsxwriter::{Format, Workbook};
use serde::{Deserialize, Serialize};

use crate::{CellValue, CockpitError, ColumnMeta, DatabaseKind, QueryResultPage, Result};

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ExportFormat {
    Txt,
    Sql,
    #[default]
    Csv,
    Excel,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ResultExportOptions {
    #[serde(default)]
    pub format: ExportFormat,
    #[serde(default)]
    pub database_name: Option<String>,
    #[serde(default)]
    pub table_name: Option<String>,
    #[serde(default)]
    pub database_kind: DatabaseKind,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CsvExportOptions {
    #[serde(default = "default_delimiter")]
    pub delimiter: u8,
    #[serde(default = "default_true")]
    pub include_headers: bool,
    #[serde(default)]
    pub write_utf8_bom: bool,
    #[serde(default = "default_true")]
    pub protect_formulas: bool,
    #[serde(default)]
    pub null_as: String,
}

fn default_delimiter() -> u8 {
    b','
}

fn default_true() -> bool {
    true
}

impl Default for CsvExportOptions {
    fn default() -> Self {
        Self {
            delimiter: default_delimiter(),
            include_headers: true,
            write_utf8_bom: false,
            protect_formulas: true,
            null_as: String::new(),
        }
    }
}

/// 逐行写入 CSV，不缓存完整结果集，适合直接接收数据库流式查询结果。
pub struct CsvStreamWriter<W: Write> {
    writer: Writer<W>,
    options: CsvExportOptions,
    rows_written: u64,
}

pub struct ResultStreamWriter<W: Write + Send> {
    inner: ResultStreamWriterInner<W>,
    rows_written: u64,
}

enum ResultStreamWriterInner<W: Write + Send> {
    Txt {
        output: W,
    },
    Sql {
        output: W,
        table_name: String,
        columns: String,
        database_kind: DatabaseKind,
    },
    Csv(Box<CsvStreamWriter<W>>),
    Excel {
        workbook: Box<Workbook>,
        output: Option<W>,
        next_row: u32,
    },
}

impl<W: Write + Send> ResultStreamWriter<W> {
    pub fn new(
        mut output: W,
        columns: &[ColumnMeta],
        options: &ResultExportOptions,
    ) -> Result<Self> {
        let inner = match options.format {
            ExportFormat::Txt => {
                output
                    .write_all(&[0xEF, 0xBB, 0xBF])
                    .map_err(exchange_error)?;
                write_txt_record(
                    &mut output,
                    columns.iter().map(|column| column.name.as_str()),
                )?;
                ResultStreamWriterInner::Txt { output }
            }
            ExportFormat::Sql => {
                output
                    .write_all(b"-- Cockpit SQL export\n")
                    .map_err(exchange_error)?;
                ResultStreamWriterInner::Sql {
                    output,
                    table_name: qualified_table_name(options),
                    columns: columns
                        .iter()
                        .map(|column| quote_identifier(&column.name, options.database_kind))
                        .collect::<Vec<_>>()
                        .join(", "),
                    database_kind: options.database_kind,
                }
            }
            ExportFormat::Csv => {
                let mut writer = CsvStreamWriter::new(
                    output,
                    CsvExportOptions {
                        write_utf8_bom: true,
                        null_as: "NULL".into(),
                        ..Default::default()
                    },
                )?;
                writer.write_headers(columns.iter().map(|column| column.name.as_str()))?;
                ResultStreamWriterInner::Csv(Box::new(writer))
            }
            ExportFormat::Excel => {
                let mut workbook = Workbook::new();
                let header_format = Format::new().set_bold();
                let worksheet = workbook.add_worksheet();
                for (column_index, column) in columns.iter().enumerate() {
                    let column_index = u16::try_from(column_index)
                        .map_err(|_| CockpitError::Exchange("Excel 列数超过限制".into()))?;
                    worksheet
                        .write_string_with_format(0, column_index, &column.name, &header_format)
                        .map_err(exchange_error)?;
                    worksheet
                        .set_column_width(column_index, 18)
                        .map_err(exchange_error)?;
                }
                ResultStreamWriterInner::Excel {
                    workbook: Box::new(workbook),
                    output: Some(output),
                    next_row: 1,
                }
            }
        };
        Ok(Self {
            inner,
            rows_written: 0,
        })
    }

    pub fn write_row(&mut self, row: &[CellValue]) -> Result<()> {
        match &mut self.inner {
            ResultStreamWriterInner::Txt { output } => {
                let options = CsvExportOptions {
                    null_as: "NULL".into(),
                    protect_formulas: false,
                    ..Default::default()
                };
                write_txt_record(output, row.iter().map(|value| cell_text(value, &options)))?;
            }
            ResultStreamWriterInner::Sql {
                output,
                table_name,
                columns,
                database_kind,
            } => {
                let values = row
                    .iter()
                    .map(|value| sql_literal(value, *database_kind))
                    .collect::<Result<Vec<_>>>()?
                    .join(", ");
                writeln!(
                    output,
                    "INSERT INTO {table_name} ({columns}) VALUES ({values});"
                )
                .map_err(exchange_error)?;
            }
            ResultStreamWriterInner::Csv(writer) => writer.write_row(row.iter())?,
            ResultStreamWriterInner::Excel {
                workbook, next_row, ..
            } => {
                if *next_row > 1_048_576 {
                    return Err(CockpitError::Exchange("Excel 行数超过限制".into()));
                }
                let worksheet = workbook.worksheet_from_index(0).map_err(exchange_error)?;
                for (column_index, value) in row.iter().enumerate() {
                    let column_index = u16::try_from(column_index)
                        .map_err(|_| CockpitError::Exchange("Excel 列数超过限制".into()))?;
                    match value {
                        CellValue::Null => {}
                        CellValue::Bool(value) => {
                            worksheet
                                .write_boolean(*next_row, column_index, *value)
                                .map_err(exchange_error)?;
                        }
                        CellValue::Float(value) if value.is_finite() => {
                            worksheet
                                .write_number(*next_row, column_index, *value)
                                .map_err(exchange_error)?;
                        }
                        _ => {
                            worksheet
                                .write_string(
                                    *next_row,
                                    column_index,
                                    truncate_excel_text(&readable_cell_text(value)),
                                )
                                .map_err(exchange_error)?;
                        }
                    }
                }
                *next_row += 1;
            }
        }
        self.rows_written += 1;
        Ok(())
    }

    pub fn rows_written(&self) -> u64 {
        self.rows_written
    }

    pub fn finish(self) -> Result<W> {
        match self.inner {
            ResultStreamWriterInner::Txt { mut output }
            | ResultStreamWriterInner::Sql { mut output, .. } => {
                output.flush().map_err(exchange_error)?;
                Ok(output)
            }
            ResultStreamWriterInner::Csv(writer) => (*writer).finish(),
            ResultStreamWriterInner::Excel {
                mut workbook,
                mut output,
                ..
            } => {
                let mut output = output
                    .take()
                    .ok_or_else(|| CockpitError::Exchange("Excel 输出已关闭".into()))?;
                workbook
                    .save_to_writer(&mut output)
                    .map_err(exchange_error)?;
                Ok(output)
            }
        }
    }
}

impl<W: Write> CsvStreamWriter<W> {
    pub fn new(mut output: W, options: CsvExportOptions) -> Result<Self> {
        if options.delimiter == b'\r' || options.delimiter == b'\n' || options.delimiter == b'"' {
            return Err(CockpitError::InvalidConfig("CSV 分隔符无效".into()));
        }
        if options.write_utf8_bom {
            output
                .write_all(&[0xEF, 0xBB, 0xBF])
                .map_err(exchange_error)?;
        }
        let writer = WriterBuilder::new()
            .delimiter(options.delimiter)
            .terminator(Terminator::CRLF)
            .from_writer(output);
        Ok(Self {
            writer,
            options,
            rows_written: 0,
        })
    }

    pub fn write_headers<'a>(&mut self, headers: impl IntoIterator<Item = &'a str>) -> Result<()> {
        if self.options.include_headers {
            self.writer.write_record(headers).map_err(exchange_error)?;
        }
        Ok(())
    }

    pub fn write_row<'a>(&mut self, values: impl IntoIterator<Item = &'a CellValue>) -> Result<()> {
        let record = values
            .into_iter()
            .map(|value| cell_text(value, &self.options))
            .collect::<Vec<_>>();
        self.writer.write_record(record).map_err(exchange_error)?;
        self.rows_written += 1;
        Ok(())
    }

    pub fn rows_written(&self) -> u64 {
        self.rows_written
    }

    pub fn finish(mut self) -> Result<W> {
        self.writer.flush().map_err(exchange_error)?;
        self.writer
            .into_inner()
            .map_err(|error| CockpitError::Exchange(error.error().to_string()))
    }
}

fn cell_text(value: &CellValue, options: &CsvExportOptions) -> String {
    let (text, protect) = match value {
        CellValue::Null => (options.null_as.clone(), false),
        CellValue::Bool(value) => (value.to_string(), false),
        CellValue::Signed(value)
        | CellValue::Unsigned(value)
        | CellValue::Decimal(value)
        | CellValue::Date(value)
        | CellValue::Time(value)
        | CellValue::DateTime(value) => (value.clone(), false),
        CellValue::Float(value) => (value.to_string(), false),
        CellValue::Text(value) | CellValue::Json(value) => (value.clone(), true),
        CellValue::Bytes {
            preview, length, ..
        } => preview
            .clone()
            .map_or_else(|| (format!("<{length} bytes>"), false), |text| (text, true)),
        CellValue::Geometry { srid, .. } => (
            srid.map_or_else(
                || "<geometry>".into(),
                |srid| format!("<geometry SRID {srid}>"),
            ),
            false,
        ),
    };
    if protect && options.protect_formulas && text.trim_start().starts_with(['=', '+', '-', '@']) {
        format!("'{text}")
    } else {
        text
    }
}

pub fn write_result_page<W: Write + Send>(
    output: W,
    page: &QueryResultPage,
    options: &ResultExportOptions,
) -> Result<()> {
    match options.format {
        ExportFormat::Txt => write_txt(output, page),
        ExportFormat::Sql => write_sql(output, page, options),
        ExportFormat::Csv => write_csv(output, page),
        ExportFormat::Excel => write_excel(output, page),
    }
}

fn write_csv<W: Write>(output: W, page: &QueryResultPage) -> Result<()> {
    let mut writer = CsvStreamWriter::new(
        output,
        CsvExportOptions {
            write_utf8_bom: true,
            null_as: "NULL".into(),
            ..Default::default()
        },
    )?;
    writer.write_headers(page.columns.iter().map(|column| column.name.as_str()))?;
    for row in &page.rows {
        writer.write_row(row.iter())?;
    }
    writer.finish().map(|_| ())
}

fn write_txt<W: Write>(mut output: W, page: &QueryResultPage) -> Result<()> {
    output
        .write_all(&[0xEF, 0xBB, 0xBF])
        .map_err(exchange_error)?;
    write_txt_record(
        &mut output,
        page.columns.iter().map(|column| column.name.as_str()),
    )?;
    let options = CsvExportOptions {
        null_as: "NULL".into(),
        protect_formulas: false,
        ..Default::default()
    };
    for row in &page.rows {
        let values = row.iter().map(|value| cell_text(value, &options));
        write_txt_record(&mut output, values)?;
    }
    output.flush().map_err(exchange_error)
}

fn write_txt_record<W, I, S>(output: &mut W, values: I) -> Result<()>
where
    W: Write,
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut first = true;
    for value in values {
        if !first {
            output.write_all(b"\t").map_err(exchange_error)?;
        }
        first = false;
        let escaped = value
            .as_ref()
            .replace('\\', "\\\\")
            .replace('\t', "\\t")
            .replace('\r', "\\r")
            .replace('\n', "\\n");
        output
            .write_all(escaped.as_bytes())
            .map_err(exchange_error)?;
    }
    output.write_all(b"\r\n").map_err(exchange_error)
}

fn write_sql<W: Write>(
    mut output: W,
    page: &QueryResultPage,
    options: &ResultExportOptions,
) -> Result<()> {
    output
        .write_all(b"-- Cockpit SQL export\n")
        .map_err(exchange_error)?;
    if page.rows.is_empty() {
        return output.flush().map_err(exchange_error);
    }
    let table_name = qualified_table_name(options);
    let columns = page
        .columns
        .iter()
        .map(|column| quote_identifier(&column.name, options.database_kind))
        .collect::<Vec<_>>()
        .join(", ");
    writeln!(output, "INSERT INTO {table_name} ({columns}) VALUES").map_err(exchange_error)?;
    for (row_index, row) in page.rows.iter().enumerate() {
        let values = row
            .iter()
            .map(|value| sql_literal(value, options.database_kind))
            .collect::<Result<Vec<_>>>()?
            .join(", ");
        let terminator = if row_index + 1 == page.rows.len() {
            ";"
        } else {
            ","
        };
        writeln!(output, "  ({values}){terminator}").map_err(exchange_error)?;
    }
    output.flush().map_err(exchange_error)
}

fn qualified_table_name(options: &ResultExportOptions) -> String {
    let table = quote_identifier(
        options.table_name.as_deref().unwrap_or("query_result"),
        options.database_kind,
    );
    options
        .database_name
        .as_deref()
        .map_or(table.clone(), |database| {
            format!(
                "{}.{table}",
                quote_identifier(database, options.database_kind)
            )
        })
}

fn quote_identifier(identifier: &str, database_kind: DatabaseKind) -> String {
    match database_kind {
        DatabaseKind::MySql | DatabaseKind::MariaDb => {
            format!("`{}`", identifier.replace('`', "``"))
        }
        DatabaseKind::PostgreSql | DatabaseKind::Sqlite | DatabaseKind::Elasticsearch => {
            format!("\"{}\"", identifier.replace('"', "\"\""))
        }
    }
}

fn sql_literal(value: &CellValue, database_kind: DatabaseKind) -> Result<String> {
    match value {
        CellValue::Null => Ok("NULL".into()),
        CellValue::Bool(value) => Ok(if *value { "TRUE" } else { "FALSE" }.into()),
        CellValue::Signed(value) | CellValue::Unsigned(value) | CellValue::Decimal(value) => {
            Ok(value.clone())
        }
        CellValue::Float(value) => Ok(if value.is_finite() {
            value.to_string()
        } else {
            "NULL".into()
        }),
        CellValue::Text(value)
        | CellValue::Date(value)
        | CellValue::Time(value)
        | CellValue::DateTime(value)
        | CellValue::Json(value) => match database_kind {
            DatabaseKind::MySql | DatabaseKind::MariaDb => Ok(mysql_utf8_literal(value)),
            DatabaseKind::PostgreSql => postgres_text_literal(value),
            DatabaseKind::Sqlite | DatabaseKind::Elasticsearch => Ok(quote_sql_string(value)),
        },
        CellValue::Bytes { base64, .. } => {
            let hex = decode_hex(base64)?;
            Ok(match database_kind {
                DatabaseKind::PostgreSql => format!("decode('{hex}', 'hex')"),
                DatabaseKind::MySql
                | DatabaseKind::MariaDb
                | DatabaseKind::Sqlite
                | DatabaseKind::Elasticsearch => format!("X'{hex}'"),
            })
        }
        CellValue::Geometry { wkb_base64, srid } => {
            let hex = decode_hex(wkb_base64)?;
            let bytes = match database_kind {
                DatabaseKind::PostgreSql => format!("decode('{hex}', 'hex')"),
                DatabaseKind::MySql
                | DatabaseKind::MariaDb
                | DatabaseKind::Sqlite
                | DatabaseKind::Elasticsearch => format!("X'{hex}'"),
            };
            Ok(srid.map_or_else(
                || format!("ST_GeomFromWKB({bytes})"),
                |srid| format!("ST_GeomFromWKB({bytes}, {srid})"),
            ))
        }
    }
}

fn mysql_utf8_literal(value: &str) -> String {
    let hex = value
        .as_bytes()
        .iter()
        .map(|byte| format!("{byte:02X}"))
        .collect::<String>();
    format!("CONVERT(X'{hex}' USING utf8mb4)")
}

/// 标准字符串字面量（SQLite / Elasticsearch SQL）：仅按标准转义单引号，
/// 不处理任何反斜杠转义，保证换行、反斜杠等字符原样往返。
fn quote_sql_string(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

/// PostgreSQL 默认开启 standard_conforming_strings，普通 '...' 不解析反斜杠；
/// 因此使用 E'' 转义字符串保留字节级内容（换行、反斜杠等）。
/// PostgreSQL 文本无法保存空字符（NUL），遇到时返回错误而不是静默损坏数据。
fn postgres_text_literal(value: &str) -> Result<String> {
    if value.contains('\0') {
        return Err(CockpitError::Exchange(
            "PostgreSQL 文本无法保存空字符（NUL）".into(),
        ));
    }
    let escaped = value
        .replace('\\', "\\\\")
        .replace('\'', "''")
        .replace('\n', "\\n")
        .replace('\r', "\\r");
    Ok(format!("E'{escaped}'"))
}

fn decode_hex(value: &str) -> Result<String> {
    let bytes = BASE64_STANDARD
        .decode(value)
        .map_err(|error| CockpitError::Exchange(format!("二进制数据解码失败：{error}")))?;
    Ok(bytes.iter().map(|byte| format!("{byte:02X}")).collect())
}

fn write_excel<W: Write + Send>(output: W, page: &QueryResultPage) -> Result<()> {
    let mut workbook = Workbook::new();
    let header_format = Format::new().set_bold();
    let worksheet = workbook.add_worksheet();
    for (column_index, column) in page.columns.iter().enumerate() {
        let column_index = u16::try_from(column_index)
            .map_err(|_| CockpitError::Exchange("Excel 列数超过限制".into()))?;
        worksheet
            .write_string_with_format(0, column_index, &column.name, &header_format)
            .map_err(exchange_error)?;
        let width = page
            .rows
            .iter()
            .filter_map(|row| row.get(column_index as usize))
            .map(readable_cell_text)
            .map(|value| value.chars().count())
            .chain([column.name.chars().count()])
            .max()
            .unwrap_or(10)
            .clamp(10, 40) as f64;
        worksheet
            .set_column_width(column_index, width)
            .map_err(exchange_error)?;
    }
    for (row_index, row) in page.rows.iter().enumerate() {
        let row_index = u32::try_from(row_index + 1)
            .map_err(|_| CockpitError::Exchange("Excel 行数超过限制".into()))?;
        for (column_index, value) in row.iter().enumerate() {
            let column_index = u16::try_from(column_index)
                .map_err(|_| CockpitError::Exchange("Excel 列数超过限制".into()))?;
            match value {
                CellValue::Null => {}
                CellValue::Bool(value) => {
                    worksheet
                        .write_boolean(row_index, column_index, *value)
                        .map_err(exchange_error)?;
                }
                CellValue::Float(value) if value.is_finite() => {
                    worksheet
                        .write_number(row_index, column_index, *value)
                        .map_err(exchange_error)?;
                }
                _ => {
                    worksheet
                        .write_string(
                            row_index,
                            column_index,
                            truncate_excel_text(&readable_cell_text(value)),
                        )
                        .map_err(exchange_error)?;
                }
            }
        }
    }
    workbook.save_to_writer(output).map_err(exchange_error)
}

fn readable_cell_text(value: &CellValue) -> String {
    cell_text(
        value,
        &CsvExportOptions {
            null_as: "NULL".into(),
            protect_formulas: false,
            ..Default::default()
        },
    )
}

fn truncate_excel_text(value: &str) -> String {
    value.chars().take(32_767).collect()
}

fn exchange_error(error: impl std::fmt::Display) -> CockpitError {
    CockpitError::Exchange(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ColumnMeta;
    use uuid::Uuid;

    fn sample_page() -> QueryResultPage {
        QueryResultPage {
            execution_id: Uuid::new_v4(),
            columns: vec![
                ColumnMeta {
                    name: "name".into(),
                    database_type: "VARCHAR".into(),
                    nullable: false,
                    unsigned: false,
                    binary: false,
                },
                ColumnMeta {
                    name: "payload".into(),
                    database_type: "BLOB".into(),
                    nullable: true,
                    unsigned: false,
                    binary: true,
                },
            ],
            rows: vec![vec![
                CellValue::Text("中文".into()),
                CellValue::Bytes {
                    base64: "AQI=".into(),
                    preview: None,
                    length: 2,
                },
            ]],
            affected_rows: 0,
            execution_time_ms: 1,
            truncated: false,
            has_more: false,
            result_set_index: 0,
            messages: vec![],
            row_offset: 0,
            page_size: 500,
            additional_result_sets: vec![],
            source_table: None,
        }
    }

    #[test]
    fn preserves_exact_numbers_and_escapes_formulas() {
        let mut writer = CsvStreamWriter::new(Vec::new(), CsvExportOptions::default()).unwrap();
        writer.write_headers(["bigint", "decimal", "text"]).unwrap();
        writer
            .write_row(
                [
                    CellValue::Unsigned("18446744073709551615".into()),
                    CellValue::Decimal("999999999999.0000000001".into()),
                    CellValue::Text("=HYPERLINK(\"https://example.test\")".into()),
                ]
                .iter(),
            )
            .unwrap();
        assert_eq!(writer.rows_written(), 1);
        let bytes = writer.finish().unwrap();
        let text = String::from_utf8(bytes).unwrap();
        assert!(text.contains("18446744073709551615"));
        assert!(text.contains("999999999999.0000000001"));
        assert!(text.contains("'=HYPERLINK"));
    }

    #[test]
    fn writes_bom_without_buffering_rows() {
        let mut writer = CsvStreamWriter::new(
            Vec::new(),
            CsvExportOptions {
                write_utf8_bom: true,
                ..Default::default()
            },
        )
        .unwrap();
        writer
            .write_row([CellValue::Text("中文".into())].iter())
            .unwrap();
        let bytes = writer.finish().unwrap();
        assert!(bytes.starts_with(&[0xEF, 0xBB, 0xBF]));
    }

    #[test]
    fn text_exports_are_utf8_and_do_not_expose_base64() {
        for format in [ExportFormat::Txt, ExportFormat::Csv] {
            let mut output = Vec::new();
            write_result_page(
                &mut output,
                &sample_page(),
                &ResultExportOptions {
                    format,
                    ..Default::default()
                },
            )
            .unwrap();
            assert!(output.starts_with(&[0xEF, 0xBB, 0xBF]));
            let text = String::from_utf8(output[3..].to_vec()).unwrap();
            assert!(text.contains("中文"));
            assert!(text.contains("<2 bytes>"));
            assert!(!text.contains("AQI="));
        }
    }

    #[test]
    fn sql_export_uses_executable_hex_for_binary_values() {
        let mut output = Vec::new();
        write_result_page(
            &mut output,
            &sample_page(),
            &ResultExportOptions {
                format: ExportFormat::Sql,
                database_name: Some("demo".into()),
                table_name: Some("items".into()),
                database_kind: DatabaseKind::MySql,
            },
        )
        .unwrap();
        let text = String::from_utf8(output).unwrap();
        assert!(text.contains("INSERT INTO `demo`.`items`"));
        assert!(text.contains("X'0102'"));
        assert!(!text.contains("AQI="));
    }

    #[test]
    fn mysql_sql_export_is_independent_of_no_backslash_escapes_mode() {
        let mut page = sample_page();
        page.rows[0][0] = CellValue::Text("quote ' slash \\ newline\n中文".into());
        let mut output = Vec::new();
        write_result_page(
            &mut output,
            &page,
            &ResultExportOptions {
                format: ExportFormat::Sql,
                database_name: Some("demo".into()),
                table_name: Some("items".into()),
                database_kind: DatabaseKind::MySql,
            },
        )
        .unwrap();
        let text = String::from_utf8(output).unwrap();
        assert!(text.contains("CONVERT(X'"));
        assert!(!text.contains("quote ' slash"));
    }

    #[test]
    fn postgres_text_literals_use_escape_strings_for_control_characters() {
        let literal = sql_literal(
            &CellValue::Text("line1\nback\\slash'quoted\r\tend".into()),
            DatabaseKind::PostgreSql,
        )
        .unwrap();
        assert_eq!(literal, "E'line1\\nback\\\\slash''quoted\\r\tend'");
    }

    #[test]
    fn sqlite_text_literals_only_double_single_quotes() {
        let literal = sql_literal(
            &CellValue::Text("line1\nback\\slash'quoted\r\tend".into()),
            DatabaseKind::Sqlite,
        )
        .unwrap();
        assert_eq!(literal, "'line1\nback\\slash''quoted\r\tend'");
    }

    #[test]
    fn elasticsearch_text_literals_only_double_single_quotes() {
        let literal = sql_literal(
            &CellValue::Text("line1\nback\\slash'quoted\r\tend".into()),
            DatabaseKind::Elasticsearch,
        )
        .unwrap();
        assert_eq!(literal, "'line1\nback\\slash''quoted\r\tend'");
    }

    #[test]
    fn postgres_text_literals_reject_null_bytes_instead_of_corrupting_data() {
        let error = sql_literal(
            &CellValue::Text("bad\u{0}byte".into()),
            DatabaseKind::PostgreSql,
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("空字符"));
        assert!(error.contains("NUL"));
    }

    #[test]
    fn excel_export_writes_a_real_xlsx_container() {
        let mut output = Vec::new();
        write_result_page(
            &mut output,
            &sample_page(),
            &ResultExportOptions {
                format: ExportFormat::Excel,
                ..Default::default()
            },
        )
        .unwrap();
        assert!(output.starts_with(b"PK"));
        assert!(output.len() > 1_000);
    }
}
