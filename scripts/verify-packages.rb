#!/usr/bin/env ruby
# frozen_string_literal: true

require "json"
require "open3"

LIBRARIES = %w[
  foundation-runtime
  point-contracts
  point-source
  render-protocol
  source-memory
  source-las
  point-index
  point-workspace
  point-view
  render-wgpu
  point-review
  point-terrain
].freeze
APPLICATIONS = %w[renderer-demo terrain-demo].freeze
VERSION = "0.12.0-alpha.1"
LICENSE = "MIT OR Apache-2.0"
MSRV = "1.90"

def capture!(*command)
  output, error, status = Open3.capture3(*command)
  return output if status.success?

  warn error
  abort "failed: #{command.join(" ")}"
end

def assert!(condition, message)
  abort message unless condition
end

root = File.expand_path("..", __dir__)
Dir.chdir(root)
metadata = JSON.parse(capture!("cargo", "metadata", "--no-deps", "--format-version", "1"))
packages = metadata.fetch("packages").to_h { |package| [package.fetch("name"), package] }

LIBRARIES.each do |name|
  package = packages.fetch(name)
  assert!(package.fetch("version") == VERSION, "#{name}: unexpected package version")
  assert!(package["publish"] != [], "#{name}: package is not publishable")
  assert!(package.fetch("license") == LICENSE, "#{name}: dual license metadata is absent")
  assert!(package.fetch("rust_version") == MSRV, "#{name}: MSRV is not #{MSRV}")
  assert!(package.fetch("repository").start_with?("https://"), "#{name}: repository is absent")
  assert!(package.fetch("homepage").start_with?("https://"), "#{name}: homepage is absent")
  assert!(package.fetch("documentation") == "https://docs.rs/#{name}", "#{name}: docs.rs URL differs")
  assert!(!package.fetch("description").strip.empty?, "#{name}: description is absent")
  assert!(!package.fetch("keywords").empty?, "#{name}: keywords are absent")
  assert!(!package.fetch("categories").empty?, "#{name}: categories are absent")
  assert!(package.fetch("features").fetch("default") == [], "#{name}: default features must be empty")
  assert!(!package.fetch("readme").nil?, "#{name}: README metadata is absent")
  manifest = File.read(package.fetch("manifest_path"))
  assert!(manifest.match?(/^publish = true$/), "#{name}: publish = true is not explicit")

  docs = package.fetch("metadata").dig("docs", "rs")
  assert!(docs == { "all-features" => true, "rustdoc-args" => ["-D", "warnings"] },
          "#{name}: docs.rs build policy differs")

  package.fetch("dependencies").each do |dependency|
    next unless LIBRARIES.include?(dependency.fetch("name"))

    assert!(dependency.fetch("req") == "=#{VERSION}",
            "#{name}: #{dependency.fetch("name")} is not pinned to =#{VERSION}")
  end

  list_arguments = ["cargo", "package", "-p", name, "--list"]
  list_arguments << "--allow-dirty" if ENV["PUNCTRA_PACKAGE_ALLOW_DIRTY"] == "1"
  entries = capture!(*list_arguments).lines.map(&:strip)
  assert!(entries.include?("README.md"), "#{name}: package README is absent")
  assert!(entries.include?("Cargo.toml"), "#{name}: normalized manifest is absent")
  assert!(entries.none? { |entry| entry.start_with?("target/") || entry.include?("examples/data/") },
          "#{name}: package contains build output or field data")
end

APPLICATIONS.each do |name|
  assert!(packages.fetch(name).fetch("publish") == [], "#{name}: application must remain private")
end

package_arguments = [
  "cargo", "package", "--workspace",
  "--exclude", "renderer-demo", "--exclude", "terrain-demo"
]
package_arguments << "--allow-dirty" if ENV["PUNCTRA_PACKAGE_ALLOW_DIRTY"] == "1"
system(*package_arguments, exception: true)
puts "verified #{LIBRARIES.length} publishable library packages at #{VERSION}"
