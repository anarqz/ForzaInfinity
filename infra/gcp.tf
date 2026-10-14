resource "google_project_service" "apis" {
  for_each = toset([
    "compute.googleapis.com",
    "iap.googleapis.com",
    "secretmanager.googleapis.com",
    "storage.googleapis.com",
  ])
  service            = each.value
  disable_on_destroy = false
}

# Our own network: the project's "default" one opens SSH and RDP to the whole internet.
resource "google_compute_network" "infinity" {
  name                    = "infinity"
  auto_create_subnetworks = false
  depends_on              = [google_project_service.apis]
}

resource "google_compute_subnetwork" "infinity" {
  name          = "infinity-${var.region}"
  network       = google_compute_network.infinity.id
  region        = var.region
  ip_cidr_range = "10.10.0.0/24"
}

# Nothing comes in from the internet: players reach the server through the Cloudflare Tunnel
# (cloudflared dials out). The only way in is SSH through Google's IAP proxy.
resource "google_compute_firewall" "iap_ssh" {
  name          = "infinity-iap-ssh"
  network       = google_compute_network.infinity.id
  direction     = "INGRESS"
  source_ranges = ["35.235.240.0/20"]
  target_tags   = ["infinity"]
  allow {
    protocol = "tcp"
    ports    = ["22"]
  }
}

resource "google_service_account" "server" {
  account_id   = "infinity-server"
  display_name = "ForzaInfinity server VM"
}

# The tunnel's token lives in Secret Manager, readable only by the VM's service account (not
# in the instance metadata, which every project viewer can read).
resource "google_secret_manager_secret" "tunnel_token" {
  secret_id = "infinity-tunnel-token"
  replication {
    auto {}
  }
  depends_on = [google_project_service.apis]
}

resource "google_secret_manager_secret_version" "tunnel_token" {
  secret      = google_secret_manager_secret.tunnel_token.id
  secret_data = data.cloudflare_zero_trust_tunnel_cloudflared_token.infinity.token
}

resource "google_secret_manager_secret_iam_member" "server_reads_token" {
  secret_id = google_secret_manager_secret.tunnel_token.id
  role      = "roles/secretmanager.secretAccessor"
  member    = "serviceAccount:${google_service_account.server.email}"
}

resource "google_storage_bucket" "backups" {
  name                        = "${var.project}-backups"
  location                    = upper(var.region)
  storage_class               = "STANDARD"
  uniform_bucket_level_access = true
  public_access_prevention    = "enforced"
  lifecycle_rule {
    condition {
      age = var.backup_days
    }
    action {
      type = "Delete"
    }
  }
  depends_on = [google_project_service.apis]
}

# The VM writes new backups; it cannot read or delete them.
resource "google_storage_bucket_iam_member" "server_writes_backups" {
  bucket = google_storage_bucket.backups.name
  role   = "roles/storage.objectCreator"
  member = "serviceAccount:${google_service_account.server.email}"
}

# The database lives on its own disk: rebuilding or replacing the VM leaves it alone, and
# Terraform refuses to destroy it. Boot (20 GB) + data (10 GB) stay within the free tier's 30.
resource "google_compute_disk" "data" {
  name = "infinity-data"
  type = "pd-standard"
  zone = var.zone
  size = 10
  lifecycle {
    prevent_destroy = true
  }
  depends_on = [google_project_service.apis]
}

# A daily snapshot of the data disk, kept 7 days (besides the SQLite backups in the bucket).
resource "google_compute_resource_policy" "daily_snapshot" {
  name   = "infinity-daily-snapshot"
  region = var.region
  snapshot_schedule_policy {
    schedule {
      daily_schedule {
        days_in_cycle = 1
        start_time    = "07:00"
      }
    }
    retention_policy {
      max_retention_days    = 7
      on_source_disk_delete = "KEEP_AUTO_SNAPSHOTS"
    }
  }
}

resource "google_compute_disk_resource_policy_attachment" "data_snapshot" {
  name = google_compute_resource_policy.daily_snapshot.name
  disk = google_compute_disk.data.name
  zone = var.zone
}

resource "google_compute_instance" "server" {
  name         = "infinity"
  machine_type = "e2-micro"
  zone         = var.zone
  tags         = ["infinity"]

  boot_disk {
    initialize_params {
      image = "debian-cloud/debian-12"
      type  = "pd-standard"
      size  = 20
    }
  }

  attached_disk {
    source      = google_compute_disk.data.id
    device_name = "infinity-data"
  }

  network_interface {
    subnetwork = google_compute_subnetwork.infinity.id
    # an ephemeral public address for the way out (packages, the tunnel); no rule lets
    # anything in through it
    access_config {}
  }

  shielded_instance_config {
    enable_secure_boot          = true
    enable_vtpm                 = true
    enable_integrity_monitoring = true
  }

  service_account {
    email  = google_service_account.server.email
    scopes = ["cloud-platform"] # what it may do is limited by its IAM roles above
  }

  metadata = {
    enable-oslogin = "TRUE"
    startup-script = templatefile("${path.module}/startup.sh", {
      backup_bucket = google_storage_bucket.backups.name
      token_secret  = google_secret_manager_secret.tunnel_token.secret_id
    })
  }

  allow_stopping_for_update = true
  depends_on                = [google_secret_manager_secret_iam_member.server_reads_token]
}
