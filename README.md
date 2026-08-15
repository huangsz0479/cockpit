**English** | [简体中文](README.zh-CN.md)

# Cockpit User Guide

Cockpit is a cross-platform desktop database client supporting MySQL, MariaDB, PostgreSQL, and SQLite. This guide is intended for everyday users and explains how to create connections, run SQL, manage data, and perform import, export, backup, and restore operations.

## 1. Supported Databases

| Database | Connection methods | Capabilities |
| --- | --- | --- |
| MySQL / MariaDB | Direct, TLS, SSH Agent, SSH private key | Queries, data editing, schema management, import/export, and backup/restore |
| PostgreSQL | Direct, TLS | Queries, data editing, object management, and backup/restore; SSH tunneling is not currently supported |
| SQLite | Local database file | No host, port, username, or password required |

Available object types and management features vary by database. Cockpit automatically shows the operations supported by the current connection.

## 2. Starting Cockpit

### Using an Installer

Download the installer for your operating system and follow the installation prompts:

- Windows: NSIS or MSI installer
- macOS: DMG installer
- Linux: AppImage or DEB package

### Starting from Source

The development environment requires Node.js `20.19+`, npm `10+`, Rust `1.88+`, and the Tauri 2 system dependencies for your platform.

~~~bash
npm install
npm run dev:tauri
~~~

The first launch downloads and compiles dependencies, so it usually takes longer than subsequent launches.

## 3. Interface Overview

The Cockpit window has three main areas:

1. Top toolbar: create a connection, create a query, open or directly execute an SQL file, or open Settings.
2. Left explorer: manage connections and browse databases, tables, views, functions, stored procedures, events, and triggers.
3. Center workspace: display the SQL editor, query results, table data, and object designers in tabs.

Connections, databases, object groups, and tables all provide context menus. Common create, refresh, backup, schema design, and delete operations are available from the context menu of the relevant object.

## 4. Creating a Database Connection

### 4.1 Create a Connection

1. Click **New Connection** in the top toolbar.
2. Select the database type.
3. Enter a connection name and connection details:
   - MySQL, MariaDB, and PostgreSQL: enter the host, port, username, and password. The default database is optional.
   - SQLite: select a local `.db`, `.sqlite`, or `.sqlite3` file.
4. Optionally assign the connection to a group.
5. Click **Test Connection** to verify the server version and connection status.
6. Click **Save Connection**.

Passwords are stored in the operating system credential store and are not written to Cockpit's local project database.

### 4.2 Safety Labels

- **Read-only connection**: blocks write statements at the driver layer. Use it when querying production data or working with a shared read-only account.
- **Production environment**: shows a prominent indicator on the connection and query tabs, helping distinguish production from test environments.

For important databases, enable the production environment label and also enable read-only mode whenever writes are unnecessary.

### 4.3 TLS and SSH

Expand **Advanced Connection Settings** to configure:

- TLS mode: Disabled, Preferred, Required, Verify CA, or Verify Hostname
- CA certificate, client certificate, and client private key
- Connection timeout, query timeout, and maximum connection pool size
- Connection color

MySQL and MariaDB also support SSH tunneling with SSH Agent or private-key authentication. Before connecting to an SSH host for the first time, verify its public-key fingerprint with the server administrator. If a saved host key changes, do not accept the new key until you understand why it changed.

### 4.4 Connect and Disconnect

- Click the arrow before a connection name in the explorer to connect and expand its resource tree.
- Click a database name to load its tables and other objects.
- Hover over a connection to use its edit and disconnect buttons.
- Right-click a connection to connect, edit, disconnect, or delete its configuration.

Deleting a connection only removes its local configuration. It does not delete any database or data from the server.

## 5. Browsing Database Objects

After connecting, the explorer displays these objects for each database:

- Tables
- Views
- Functions and stored procedures
- Events
- Triggers

Common operations:

- Double-click a table or press `Enter` to preview its data.
- Right-click a table to preview the first 100 rows, generate a `SELECT`, design its schema, copy its name, truncate it, or drop it.
- Double-click a view, function, stored procedure, event, or trigger to view its definition.
- Right-click an object group to create an object, create a query, or refresh the list.
- Right-click a database to open or refresh it, create a backup, restore an SQL backup, or compare schemas.

UI actions that truncate a table, drop a table, or drop a database cannot be undone and require a second confirmation.

## 6. Using the SQL Workspace

### 6.1 Create or Open a Query

- Click **New Query** in the top toolbar to create an empty SQL tab.
- Click **Open SQL** to load a local `.sql` file.
- Click **Run SQL** to select and execute a local `.sql` file against the selected database without loading it into the editor or showing a second confirmation.
- SQL files are read as a stream. The lower-right task card shows processed bytes, percentage, and executed statements, and allows cancellation. Memory use is bounded mainly by the largest individual statement; a single extremely large `INSERT` still has to be retained while it executes.
- Select the connection and database to use from the query toolbar.
- Click **Save** to save the current SQL. The first save prompts you to choose a file location.

When workspace autosave is enabled, Cockpit restores query tabs, SQL content, and recent workspace state. Workspace content that has not been saved to a file is not a substitute for a proper backup.

### 6.2 Write and Run SQL

The editor provides syntax highlighting, completion, and formatting for the current database:

1. Enter SQL and verify the connection and database in the query toolbar.
2. To run only part of the content, select the target SQL first. Both the **Run** button and the keyboard shortcut execute only the selected text; when no text is selected, Cockpit runs all SQL in the editor.
3. Click **Run**, or press `Ctrl+Enter`; on macOS, press `⌘+Enter`.
4. Review data, affected-row counts, execution information, or multiple result sets in the results area below.
5. Click **Stop** to cancel a long-running query.

If the SQL contains parameter placeholders, Cockpit prompts you to enter values before execution.

### 6.3 Transactions

Click **Transaction** in the query toolbar to start a transaction for the current tab. After a transaction begins:

- Click **Commit** to save changes permanently.
- Click **Rollback** to undo changes made in the current transaction.
- Cockpit prompts you to resolve uncommitted transactions when closing the tab, switching connections, or disconnecting.

Each query or data tab uses an independent session. A transaction in one tab does not automatically apply to another tab.

### 6.4 Query Results

The results area supports:

- Multiple result sets and pagination
- Search, sorting, and filtering on the current page
- Column resizing, hiding, reordering, and freezing
- Result analysis and numeric summaries
- Copying the current row or current page
- Exporting the current page or the complete query result

Not every SQL statement can be safely converted into a fully paginated query. In those cases, only the current page can be exported.

## 7. Viewing and Editing Table Data

Double-click a table in the resource tree to open it in a data tab.

### 7.1 Filtering and Pagination

- Use the toolbar search to quickly filter the current page.
- Use column-header menus to sort or filter.
- Use the Previous and Next buttons in the lower-right corner to move between pages.
- Change the table page size in Settings to control how many rows are loaded per page.

Page search only applies to the currently loaded page. To narrow results on the server, use column filters or write a query with a `WHERE` clause.

### 7.2 Add and Edit Rows

- Click **Add**, enter values in the new row, press `Enter` to save, or press `Esc` to cancel.
- Select a row and click **Edit** to change cells and save the row.
- Select multiple rows and choose **Bulk Edit** from the **More** menu.
- Nullable fields can be explicitly set to `NULL`. Fields with database defaults can be left at their default values.

Row editing requires a primary key or unique key. When saving, Cockpit also checks the original values. If another session has already changed the row, Cockpit stops the write and reports a conflict.

### 7.3 Delete Rows

- Select a row and click **Delete** to remove it.
- Select multiple rows and choose **Bulk Delete** from the **More** menu.

Cockpit attempts to perform bulk writes in a transaction and rolls back changes from the same batch when an error occurs. Even so, verify the selected connection, database, and row count before deleting.

### 7.4 Special Data Types

- JSON content can be viewed with formatting.
- Binary fields can be previewed or saved to a file.
- Date and time fields provide appropriate input controls.
- High-precision numbers are displayed as exact text to avoid precision loss from frontend floating-point conversion.

## 8. Table Schemas and Database Objects

### 8.1 Create a Table

Right-click the **Tables** group under a database and select **New Table**. In the designer, enter the table name, columns, types, nullability, default values, primary key, indexes, foreign keys, and other details. Preview and run the generated SQL when ready.

For PostgreSQL, Cockpit opens prefilled `CREATE TABLE` SQL that you can adjust before running it.

### 8.2 Modify a Table Schema

Right-click an existing table and select **Design Table Schema**. After changing columns or indexes, review the generated `ALTER SQL` before running it.

### 8.3 Other Objects

Use the context menu on the relevant group to create or edit views, functions, stored procedures, triggers, and events. SQLite does not support stored procedures, functions, or events. PostgreSQL does not support MySQL `EVENT` objects.

### 8.4 Schema Comparison

Right-click a database, select **Schema Comparison**, and choose another database as the target. Cockpit displays schema differences and generates migration and rollback SQL. Validate the generated output in a test environment before running it.

## 9. Import and Export

### 9.1 Import Table Data

1. Open the target table.
2. Click **More** and select **Import Data**.
3. Select a CSV, TSV, TXT, XLSX, XLS, or XLSB file.
4. For CSV files, verify the encoding and delimiter. For Excel files, select a worksheet.
5. Review the source-to-target column mapping. Set columns you do not need to Ignore.
6. Choose a conflict strategy: Stop on Error, Ignore Duplicate Keys, Replace Entire Row, or Update Duplicate Keys.
7. Set the batch size and `NULL` marker, then start the import.

Before importing, carefully review data types, unique keys, and the conflict strategy. When using Replace Entire Row, values not provided in the file may be affected by the database's replacement semantics.

### 9.2 Export Data

Cockpit supports TXT, SQL, CSV, and Excel (`.xlsx`) exports:

- Query results: click **Export** in the query toolbar and choose the current page or all results.
- Table data: open the table and choose the current page or entire table from the export section of the **More** menu.

Progress is displayed for large exports. By default, Cockpit protects CSV values beginning with `= + - @` from being interpreted as spreadsheet formulas, reducing the risk of formula execution when the file is opened in Excel or similar applications.

### 9.3 Import an SQL File

Right-click the target database, select **Restore SQL Backup**, and choose an SQL file. Restoring executes statements from the file and may overwrite or delete existing objects and data. Back up the current database first.

## 10. Backup and Restore

### 10.1 Manual Backup

1. Under **Backup & Export** in Settings, choose whether to include table data, use Gzip compression, or enable encryption.
2. Right-click the target database and select **Back Up Database**.
3. Choose an output location.
4. If encryption is enabled, enter a backup password of at least eight characters.

When the backup finishes, Cockpit displays the numbers of tables, objects, and data rows and generates a SHA-256 checksum. Encryption uses AES-256-GCM. The password is not stored, and a lost password cannot be recovered.

### 10.2 Restore a Backup

Right-click the target database and select **Restore SQL Backup**. If the backup is compressed or encrypted, Cockpit handles it during restoration. An encrypted backup requires the password used when it was created.

Before restoring:

1. Create a fresh backup of the current database.
2. Verify that the current connection is not read-only.
3. Verify that the backup's database type matches the target database.
4. Test the restore in a non-production environment before restoring production data.

### 10.3 Scheduled Backups

Scheduled backups only run while Cockpit is open. If the application is closed, the target connection is disconnected, or the computer is asleep, Cockpit cannot guarantee that a schedule will run like a system background service. Important production backups should also use server-level or operating-system-level backup solutions.

## 11. Settings and Diagnostics

Click **Settings** in the top toolbar to configure:

- Query and table page sizes
- Whether system databases are shown
- Workspace autosave
- Editor font size and tab width
- Default export format, backup content, compression, and encryption
- GitHub Releases update checks on startup

Use **Diagnostic Logs** in Settings to investigate connection or execution issues. Logs redact sensitive connection information, but you should still check for business table names, SQL, or other internal information before sharing them.

## 12. Keyboard Shortcuts

Windows and Linux use `Ctrl`; macOS uses `⌘`.

| Action | Windows / Linux | macOS |
| --- | --- | --- |
| New query | `Ctrl+N` | `⌘+N` |
| Open SQL file | `Ctrl+O` | `⌘+O` |
| Run SQL file directly | `Ctrl+Shift+O` | `⌘+Shift+O` |
| Save current SQL | `Ctrl+S` | `⌘+S` |
| Save current SQL as | `Ctrl+Shift+S` | `⌘+Shift+S` |
| Run the selection; when there is no selection, run all SQL | `Ctrl+Enter` | `⌘+Enter` |
| Close current tab | `Ctrl+W` | `⌘+W` |
| Open Settings | `Ctrl+,` | `⌘+,` |
| Close current dialog or menu | `Esc` | `Esc` |

## 13. Security Recommendations

- Enable the **Production Environment** label for production connections. Prefer a **Read-only Connection** for routine queries.
- Direct SQL execution—including `UPDATE`, `DELETE`, and DDL—SQL files, and restore jobs do not show a second confirmation. Verify the SQL, current connection, and target database first.
- Delete and truncate actions started from the explorer, data grid, or table designer still require a second confirmation.
- In SQL, `UPDATE` / `DELETE` statements without a `WHERE` clause and `DROP` / `TRUNCATE` statements run immediately. Validate them in a transaction or test environment first.
- Create a usable backup before importing, restoring, migrating schemas, or making bulk changes.
- Do not put connection passwords, SSH private keys, or backup passwords in SQL files, screenshots, or issue reports.

## 14. Troubleshooting

### Cannot Connect to a Database

Check the host and port, username and password, database server bind address, firewall, account host restrictions, and TLS configuration. For SSH connections, also verify the bastion host address, SSH user, Agent or private key availability, and host-key fingerprint.

### Queries Work, but Data Cannot Be Modified

Check whether the connection is read-only, the database account has write permissions, the table has a primary or unique key, and there is an unfinished transaction. Cockpit does not allow row editing or deletion when it cannot reliably identify a row.

### Complete Export Is Unavailable

The current SQL may not support safe pagination. Try a straightforward single-table `SELECT`, narrow the query and export it page by page, or open the table directly and export the entire table.

### SSH Settings Are Missing for PostgreSQL

The current PostgreSQL driver supports direct and TLS connections but not SSH tunneling. You can create port forwarding outside Cockpit and connect Cockpit to the local forwarded port.

### A Scheduled Backup Did Not Run

Scheduled backups require Cockpit to remain open and the target connection to stay connected. They are not operating-system background tasks and do not continue after the application exits.

## 15. Known Limitations

- PostgreSQL does not currently support SSH tunneling.
- Cockpit checks stable releases on GitHub and displays a notification when a newer version is available. Downloads and installation remain manual.
- Scheduled backups only run while Cockpit is open.
- CSV and Excel imports retain parsed data in memory until submission. Split very large files into smaller batches.
- Some metadata or management features may be unavailable depending on the database version and account permissions.

## 16. Building and Releasing

For developers:

- Local builds: `npm run build` (frontend), `cargo build` (Rust workspace), `npm run tauri build` (full desktop installers).
- Release: bump the version in `package.json`, `src-tauri/tauri.conf.json`, and the root `Cargo.toml` (`[workspace.package] version`) to the same value, then push a `vX.Y.Z` tag. `.github/workflows/release.yml` builds installers for macOS / Windows / Linux and publishes them to GitHub Releases, verifying the tag matches `tauri.conf.json`. macOS signing/notarization depends on repository secrets; without them, an unsigned bundle is produced.
- Preview installers: run the **Desktop installers** workflow (`.github/workflows/windows-package.yml`) manually from the Actions page.
- Updates: Cockpit checks GitHub Releases for stable versions on startup and only shows a notification; there is no automatic download or installation.

## License

Cockpit is released under the [Apache License 2.0](LICENSE).
