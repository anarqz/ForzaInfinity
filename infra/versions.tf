terraform {
  required_version = ">= 1.9"
  required_providers {
    google = {
      source  = "hashicorp/google"
      version = ">= 6.0"
    }
    random = {
      source  = "hashicorp/random"
      version = ">= 3.6"
    }
    cloudflare = {
      source  = "cloudflare/cloudflare"
      version = "~> 5.0"
    }
  }
}

# Credentials come from the environment (see tf.sh): GOOGLE_OAUTH_ACCESS_TOKEN from gcloud and
# CLOUDFLARE_API_TOKEN from ~/.config/infinity-cloudflare.env.
provider "google" {
  project = var.project
  region  = var.region
  zone    = var.zone
}

provider "cloudflare" {}
