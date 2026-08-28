#!/usr/bin/env ruby
# frozen_string_literal: true

require "json"
require "open3"
require "tmpdir"

EXPECTED_LIBRARY_COUNT = 12
EXPECTED_APPLICATION_COUNT = 3

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
workspace_members = metadata.fetch("workspace_members")
workspace_packages = metadata.fetch("packages").select do |package|
  workspace_members.include?(package.fetch("id"))
end
libraries = workspace_packages.reject { |package| package.fetch("publish") == [] }
applications = workspace_packages.select { |package| package.fetch("publish") == [] }

assert!(libraries.length == EXPECTED_LIBRARY_COUNT,
        "expected #{EXPECTED_LIBRARY_COUNT} publishable libraries, found #{libraries.length}")
assert!(applications.length == EXPECTED_APPLICATION_COUNT,
        "expected #{EXPECTED_APPLICATION_COUNT} private applications, found #{applications.length}")

library_names = libraries.to_h { |package| [package.fetch("name"), package] }
ordered_names = []
pending_names = library_names.keys.sort
until pending_names.empty?
  ready = pending_names.select do |name|
    library_names.fetch(name).fetch("dependencies").all? do |dependency|
      !library_names.key?(dependency.fetch("name")) || ordered_names.include?(dependency.fetch("name"))
    end
  end
  abort "publishable library dependency cycle: #{pending_names.join(", ")}" if ready.empty?

  ordered_names.concat(ready)
  pending_names -= ready
end
libraries = ordered_names.map { |name| library_names.fetch(name) }

versions = workspace_packages.map { |package| package.fetch("version") }.uniq
licenses = libraries.map { |package| package.fetch("license") }.uniq
msrvs = libraries.map { |package| package.fetch("rust_version") }.uniq
assert!(versions.length == 1, "workspace package versions differ: #{versions.join(", ")}")
assert!(licenses.length == 1 && !licenses.first.to_s.empty?, "library license policy differs")
assert!(msrvs.length == 1 && !msrvs.first.to_s.empty?, "library MSRV policy differs")
version = versions.fetch(0)
license = licenses.fetch(0)
msrv = msrvs.fetch(0)

libraries.each do |package|
  name = package.fetch("name")
  assert!(package.fetch("version") == version, "#{name}: unexpected package version")
  assert!(package["publish"] != [], "#{name}: package is not publishable")
  assert!(package.fetch("license") == license, "#{name}: license metadata differs")
  assert!(package.fetch("rust_version") == msrv, "#{name}: MSRV differs")
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
    next unless library_names.key?(dependency.fetch("name"))

    assert!(dependency.fetch("req") == "=#{version}",
            "#{name}: #{dependency.fetch("name")} is not pinned to =#{version}")
  end

  list_arguments = ["cargo", "package", "-p", name, "--list"]
  list_arguments << "--allow-dirty" if ENV["PUNCTRA_PACKAGE_ALLOW_DIRTY"] == "1"
  entries = capture!(*list_arguments).lines.map(&:strip)
  assert!(entries.include?("README.md"), "#{name}: package README is absent")
  assert!(entries.include?("Cargo.toml"), "#{name}: normalized manifest is absent")
  assert!(entries.none? { |entry| entry.start_with?("target/") || entry.include?("examples/data/") },
          "#{name}: package contains build output or field data")
end

applications.each do |package|
  name = package.fetch("name")
  assert!(package.fetch("publish") == [], "#{name}: application must remain private")
  manifest = File.read(package.fetch("manifest_path"))
  assert!(manifest.match?(/^publish = false$/), "#{name}: publish = false is not explicit")
end

package_arguments = ["cargo", "package", "--workspace", "--no-verify"]
applications.each do |package|
  package_arguments.concat(["--exclude", package.fetch("name")])
end
package_arguments << "--allow-dirty" if ENV["PUNCTRA_PACKAGE_ALLOW_DIRTY"] == "1"
package_root = File.join(root, "target", "package")
system(*package_arguments, exception: true)

Dir.mktmpdir("punctra-package-verification-") do |verification_root|
  members = libraries.map do |package|
    name = package.fetch("name")
    member = "#{name}-#{version}"
    archive = File.join(package_root, "#{member}.crate")
    system("tar", "-xzf", archive, "-C", verification_root, exception: true)
    member
  end
  patches = libraries.map do |package|
    name = package.fetch("name")
    %(#{name} = { path = #{("./#{name}-#{version}").dump} })
  end
  verifier = <<~TOML
    [workspace]
    resolver = "3"
    members = #{JSON.generate(members)}

    [patch.crates-io]
    #{patches.join("\n")}
  TOML
  File.write(File.join(verification_root, "Cargo.toml"), verifier)
  verification_target = File.join(root, "target", "package-verification")
  system(
    { "CARGO_TARGET_DIR" => verification_target },
    "cargo", "check", "--workspace", "--all-features",
    chdir: verification_root,
    exception: true,
  )
  system(
    { "CARGO_TARGET_DIR" => verification_target },
    "cargo", "test", "-p", "render-wgpu", "--all-features", "--no-run",
    chdir: verification_root,
    exception: true,
  )
end
puts "verified #{libraries.length} publishable library packages at #{version}"
