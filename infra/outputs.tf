output "server_url" {
  value = "https://${var.hostname}"
}

output "instance" {
  value = google_compute_instance.server.name
}

output "backup_bucket" {
  value = google_storage_bucket.backups.name
}
