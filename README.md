# Project Setup & Run Guide

## Rust migration runtime

The in-progress unified Rust service is built from this repository root. The legacy services remain available while parity work tracked in `docs/migration-matrix.md` continues.

```bash
cargo build
cargo test
cargo run -- --bind 127.0.0.1:3000
```

Safe defaults enable the API shell, health/readiness, Prometheus metrics, and Swagger on one listener. Enable Keycloak-backed authentication by supplying the required secret through environment or a protected TOML file:

```bash
FARMCONTROLLER__AUTH__CLIENT_SECRET='...' \
  cargo run -- --enable auth --config config/default.toml
```

Operational routes are `/health`, `/ready`, `/metrics`, `/openapi.json`, and `/swagger-ui/`. Runtime modules may be repeated or comma-separated with `--enable` and `--disable`. Configuration precedence is defaults, TOML file, environment, then CLI.

Production Linux and container paths are now available without Node.js or Python backend processes:

```bash
cargo build --locked --release
docker compose -f docker-compose.rust.yml up --build
```

See `docs/architecture.md`, `docs/configuration.md`, `docs/development.md`, `docs/deployment.md`, `docs/observability.md`, `docs/migration.md`, `docs/compatibility.md`, and `docs/troubleshooting.md`. The legacy instructions below remain only for parallel validation and rollback until the migration checklist is closed.

## Prerequisites

- [Docker](https://www.docker.com/get-started) installed
- [Node.js](https://nodejs.org/) and [npm](https://www.npmjs.com/) installed (for local scripts)
- [Keycloak](http://localhost:9000). To be started and automatically configuration along with the FC server.
- NFS Server installed and configured (for test execution artifacts)

## Setup Instructions

### 1. Automated Setup (Recommended)

All prerequisites (Node.js, Docker, NFS) can be installed and configured automatically using the deploy script:

```bash
cd deploy
sudo bash deploy.sh
```

This will:

- Install Node.js (via NVM) at the version specified in `deploy/.env`
- Install Docker & Docker Compose
- Install and configure NFS server with `/nfs_share` export
- Install and configure Nginx file server (artifacts & test results on port 8080)
- Skip any step that is already configured

> Requires **Ubuntu 22.04+** and **sudo** privileges.

### 2. Run with Docker for Production mode

To build and start all services:

```bash
docker compose up --build
```

### 3. Start the Development Server

Navigate to the root project directory, install Node.js dependencies, and run the development server locally:

```bash
npm install
npm run dev
```

This now also starts the `reports-service` as a Docker container (on port `5003`) along with the existing Node.js services.
All Python dependencies are installed inside the Docker image — no local Python or `pip` setup is required.

### 4. Reports Service

The `reports-service` runs entirely via Docker — no local Python setup is needed.

It requires the `/test-results` directory on the host, which is configured as part of the [File Server Configuration (Artifacts & Test Results)](#file-server-configuration-artifacts--test-results) section.

Local reports-service port: `5003`
Docker reports-service internal port: `8083`

## Notes

- Use `docker compose down` to stop and remove all running containers.
- Production `docker compose up --build` now includes the `reports` service container.
- `reports-service` runs as a Docker container and listens on `5003` locally (`npm run dev`), and on `8083` inside Docker. No local Python setup is needed.
- Ensure all environment variables are correctly set in a `.env` file if required.
- This project uses **npm workspaces** for managing multiple packages. Running `npm install` in the root folder installs all workspace dependencies.
- For debugging or local development, no need to run `npm install` in individual packages—just run it once at the root.
- The NFS share at `/nfs_share` must be accessible and writable for test execution artifacts.
- Verify NFS server status with `showmount -e localhost` after setup.

# TestRail & GitLab Integration

### TestRail Integration

This project integrates with [TestRail](https://www.testrail.com/) for managing test plans, test suites, and test cases.

**TestRail Environment Variables:**

- `TEST_RAIL_BASE_URL`: The base URL for your TestRail instance (e.g., `https://yourcompany.testrail.io/index.php`)
- `TEST_RAIL_USERNAME`: Your TestRail username/email for authentication
- `TEST_RAIL_API_KEY`: Your TestRail API key for authentication
- `TEST_RAIL_API_VERSION`: The TestRail API version (typically `v2`)
- `TEST_RAIL_PROJECT_ID`: The TestRail project ID containing your test cases

Example from `.env`:

```env
TEST_RAIL_BASE_URL=https://yourcompany.testrail.io/index.php
TEST_RAIL_USERNAME=your.email@company.com
TEST_RAIL_API_KEY=your_testrail_api_key
TEST_RAIL_API_VERSION=v2
TEST_RAIL_PROJECT_ID=1
```

### GitLab Integration

Test case files (scripts) are stored in a GitLab repository. The system fetches these files using the following environment variables:

- `GITLAB_BASE_URL`: The base URL for your GitLab instance (e.g., `https://gitlab.com/api/v4`)
- `GITLAB_PROJECT_ID`: The GitLab project ID containing the test scripts
- `GITLAB_ACCESS_TOKEN`: Personal access token for GitLab API access
- `GITLAB_BRANCH`: The branch from which to fetch test case files

Example from `.env`:

```env
GITLAB_BASE_URL=https://gitlab.com/api/v4
GITLAB_PROJECT_ID=2222
GITLAB_ACCESS_TOKEN=your_gitlab_access_token
GITLAB_BRANCH=main
```

---

## Test Plan, Test Suite, and Test Case Flow

- **Test Plan:**  
  A high-level grouping of related test suites. Represents a full validation or feature set (e.g., "Release 1.0 Regression").

- **Test Suite:**  
  A logical collection of test cases within a test plan, often grouped by feature, module, or scenario (e.g., "Login Functionality Suite").

- **Test Case:**  
  An individual test with a description and a script file name.
  - The **file name** must match the script file stored in the GitLab repository.
  - The **description** explains the purpose and steps of the test.

**Example Structure:**

```
Test Plan: "Release 1.0 Regression"
  └── Test Suite: "Login Functionality Suite"
        ├── Test Case: "Valid Login"
        │     - Description: "Verify login with valid credentials"
        │     - File Name: "valid_login.sh"
        ├── Test Case: "Invalid Login"
        │     - Description: "Verify login fails with invalid credentials"
        │     - File Name: "invalid_login.sh"
  └── Test Suite: "Signup Functionality Suite"
        └── Test Case: ...
```

- **Descriptions** for each entity:
  - **Test Plan:** Describes the overall goal or scope of the testing effort.
  - **Test Suite:** Describes the feature or module being tested.
  - **Test Case:** Describes the specific scenario, expected outcome, and references the script file (must match GitLab).

---

**Note:**

- Ensure that the test case file names in TestRail exactly match the script files in the GitLab repository.

## File Server Configuration (Artifacts & Test Results)

Nginx is installed and configured automatically by the deploy script (`sudo bash deploy.sh`). It serves two directories on port `8080`:

- **Artifacts** — build artifacts at `/artifacts/`
- **Test Results** — test result files at `/test-results/`

After running the deploy script, the following are set up automatically:

- Nginx site config at `/etc/nginx/sites-available/artifacts-local`
- Directories `/artifacts` and `/test-results` with correct permissions (`user:www-data`, mode `755`)
- Nginx reloaded and listening on port `8080`

#### Environment Variables

Add the following to the relevant `.env` files:

**`device-service/.env`** — used by the report generation service to resolve the test results directory:

```env
TEST_RESULTS_DIR=/test-results
```

**`frontend/.env`** — used by the frontend to construct direct download URLs for test case log files:

```env
VITE_TEST_RESULTS_BASE_URL=http://<your-server>:8080
```

Replace `<your-server>` with your server's hostname or IP address.

---

# 📂 Artifacts Folder Configuration

This guide explains how to configure artifacts folders for devices using the available API endpoints.

You can use **Postman**, **Insomnia**, or plain **cURL** commands to perform these steps.
👉 Make sure you are logged in as an **admin** before configuring artifacts.

---

## 🔑 Step 1: Admin Login

Use the **signin** API to log in as `dev-admin` (or any admin account).

```bash
curl --location 'http://localhost:5000/api/v1/auth/signin' \
--header 'Content-Type: application/json' \
--data '{
    "emailOrUsername": "dev-admin",
    "password": "password"
}'
```

The response will include **cookies** (`accessToken` and `refreshToken`).
These cookies are required for subsequent API calls.

---

## 📂 Step 2: Configure Artifacts Folder

You can configure artifacts **in bulk** or **single entry**.
There are **two ways** to pass cookies:

---

### 🔹 Option 1: Manual Cookie Header

Copy `accessToken` and `refreshToken` from login response and include them in each curl request.

#### ✅ Bulk Configuration

```bash
curl --location --request PUT 'http://localhost:5000/api/v1/device/config/artifacts' \
--header 'Content-Type: application/json' \
--header 'Cookie: accessToken=<your-access-token>; refreshToken=<your-refresh-token>' \
--data '[
  { "deviceType": "h3", "folderName": "Gen3_h3", "defaultVersion": "v2.0.0", "deviceFamily": "Gen3" },
  { "deviceType": "m3ne", "folderName": "Gen3_m3ne", "defaultVersion": "v2.0.0", "deviceFamily": "Gen3" },
  { "deviceType": "m3le", "folderName": "Gen3_m3le", "defaultVersion": "v2.0.0", "deviceFamily": "Gen3" },
  { "deviceType": "m3-w", "folderName": "Gen3_m3-w", "defaultVersion": "v2.0.0", "deviceFamily": "Gen3" },
  { "deviceType": "v4h", "folderName": "XOS3", "defaultVersion": "v3.34.0", "deviceFamily": "Gen4" }
]'
```

#### ✅ Single Configuration

```bash
curl --location --request PUT 'http://localhost:5000/api/v1/device/config/artifacts' \
--header 'Content-Type: application/json' \
--header 'Cookie: accessToken=<your-access-token>; refreshToken=<your-refresh-token>' \
--data '{
  "deviceType": "h3",
  "folderName": "Gen3_h3",
  "defaultVersion": "v2.0.0"
}'
```

---

### 🔹 Option 2: Auto-Save & Reuse Cookies (`cookie.txt`)

Instead of copying tokens manually, let curl **store cookies in a file** during login, then reuse them for subsequent requests.

#### Step 1 → Login & Save Cookies

```bash
curl --location 'http://localhost:5000/api/v1/auth/signin' \
--header 'Content-Type: application/json' \
--data '{
    "emailOrUsername": "dev-admin",
    "password": "password"
}' \
-c cookie.txt
```

> This saves the `accessToken` and `refreshToken` into `cookie.txt`.

#### Step 2 → Use Cookies for Requests

Now just add `-b cookie.txt` in subsequent API calls:

**Bulk Configuration**

```bash
curl --location --request PUT 'http://localhost:5000/api/v1/device/config/artifacts' \
--header 'Content-Type: application/json' \
--data '[
   { "deviceType": "h3", "folderName": "Gen3_h3", "defaultVersion": "v2.0.0", "deviceFamily": "Gen3" },
  { "deviceType": "m3ne", "folderName": "Gen3_m3ne", "defaultVersion": "v2.0.0", "deviceFamily": "Gen3" },
  { "deviceType": "m3le", "folderName": "Gen3_m3le", "defaultVersion": "v2.0.0", "deviceFamily": "Gen3" },
  { "deviceType": "m3-w", "folderName": "Gen3_m3-w", "defaultVersion": "v2.0.0", "deviceFamily": "Gen3" },
  { "deviceType": "v4h", "folderName": "XOS3", "defaultVersion": "v3.34.0", "deviceFamily": "Gen4" }
]' \
-b cookie.txt
```

**Single Configuration**

```bash
curl --location --request PUT 'http://localhost:5000/api/v1/device/config/artifacts' \
--header 'Content-Type: application/json' \
--data '{
  "deviceType": "h3",
  "folderName": "Gen3_h3",
  "defaultVersion": "v2.0.0"
}' \
-b cookie.txt
```

---

## ⚙️ Step 3 (Optional): Copy Default Artifacts

This API copies the **default artifacts** to **NFS** and **TFTP**.

- It accepts a `force` parameter.
- If `force=true`, old defaults are removed and replaced.

### With Manual Cookie Header

```bash
curl --location --request PUT 'http://localhost:5000/api/v1/device/config/artifacts/default?force=false' \
--header 'Content-Type: application/json' \
--header 'Cookie: accessToken=<your-access-token>; refreshToken=<your-refresh-token>' \
--data '{}'
```

### With Cookie File

```bash
curl --location --request PUT 'http://localhost:5000/api/v1/device/config/artifacts/default?force=false' \
--header 'Content-Type: application/json' \
--data '{}' \
-b cookie.txt
```

---

## ✅ Summary

- **Step 1** → Login as admin.
  - Option 1: Copy tokens manually and use `Cookie` header.
  - Option 2: Use `-c cookie.txt -b cookie.txt` to save & reuse cookies automatically.

- **Step 2** → Configure artifacts folder (bulk or single).
- **Step 3** (optional) → Copy default artifacts to NFS and TFTP.

---
