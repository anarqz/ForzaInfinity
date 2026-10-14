variable "project" {
  type    = string
  default = "forza-infinity-online"
}

# us-east1 / us-west1 / us-central1 are the regions of the e2-micro free tier.
variable "region" {
  type    = string
  default = "us-east1"
}

variable "zone" {
  type    = string
  default = "us-east1-b"
}

variable "domain" {
  description = "The Cloudflare zone."
  type        = string
  default     = "anarqz.dev"
}

variable "hostname" {
  description = "The server's public name (the players' Server=https://...)."
  type        = string
  default     = "infinity.anarqz.dev"
}

variable "backup_days" {
  description = "Days a database backup is kept."
  type        = number
  default     = 30
}
