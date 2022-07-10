use base64::URL_SAFE_NO_PAD;
use blake3::Hash;
use monostate::MustBe;
use semver::Version;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::{BTreeMap, BTreeSet};

struct HashDef;

impl HashDef {
	fn deserialize<'de, D>(deserializer: D) -> Result<Hash, D::Error>
	where
		D: Deserializer<'de>
	{
		let parts = <(u64, u64, u64, u64)>::deserialize(deserializer)?;
		let mut hash = [0u8; 32];
		hash[0..8].clone_from_slice(&parts.0.to_be_bytes());
		hash[8..16].clone_from_slice(&parts.1.to_be_bytes());
		hash[16..24].clone_from_slice(&parts.2.to_be_bytes());
		hash[24..32].clone_from_slice(&parts.3.to_be_bytes());
		Ok(hash.into())
	}

	fn serialize<S>(this: &Hash, serializer: S) -> Result<S::Ok, S::Error>
	where
		S: Serializer
	{
		let hash = this.as_bytes();
		let parts = (
			u64::from_be_bytes((&hash[0..8]).try_into().unwrap()),
			u64::from_be_bytes((&hash[8..16]).try_into().unwrap()),
			u64::from_be_bytes((&hash[16..24]).try_into().unwrap()),
			u64::from_be_bytes((&hash[24..32]).try_into().unwrap())
		);
		parts.serialize(serializer)
	}
}

#[derive(Deserialize, Eq, PartialEq, PartialOrd, Ord, Serialize)]
#[rustfmt::skip]
struct Dependency(
	String,
	Option<Version>,
	#[serde(skip_serializing_if = "Option::is_none", default)]
	Option<String>
);

impl Dependency {
	fn new(crate_name: String, version: Option<Version>, lib_name: String) -> Self {
		let lib_name = (lib_name != crate_name).then(|| lib_name);
		Self(crate_name, version, lib_name)
	}

	fn crate_name(&self) -> &str {
		&self.0
	}

	fn version(&self) -> Option<&Version> {
		self.1.as_ref()
	}

	fn lib_name(&self) -> &str {
		self.2.as_deref().unwrap_or_else(|| self.crate_name())
	}
}

#[derive(Deserialize, Serialize)]
struct DependencyInfoV1 {
	/// The version of the markdown output. If there are significant changes made to the
	/// markdown output that require to re-run this tool eventhough none of the inputs
	/// has changed, this version should be increased.
	#[serde(rename = "m")]
	markdown_version: u8,

	/// The blake3 hash of the template file.
	#[serde(rename = "t", with = "HashDef")]
	template_hash: Hash,

	/// The blake3 hash of the input rustdoc.
	#[serde(rename = "r", with = "HashDef")]
	rustdoc_hash: Hash,

	/// The versions of dependencies that are used for link generation. The first entry
	/// of the tuple is the dependency name on crates.io, the second is the version,
	/// and the third is the dependency name as seen in Rust code (or missing if it is
	/// equivalent to the dependency name on crates.io).
	#[serde(rename = "d")]
	dependencies: BTreeSet<Dependency>
}

#[derive(Deserialize, Serialize)]
#[serde(untagged)]
enum DependencyInfoImpl {
	V1(MustBe!(1u8), DependencyInfoV1)
}

impl DependencyInfoImpl {
	fn new(markdown_version: u8, template: &str, rustdoc: &str) -> Self {
		Self::V1(Default::default(), DependencyInfoV1 {
			markdown_version,
			template_hash: blake3::hash(template.as_bytes()),
			rustdoc_hash: blake3::hash(rustdoc.as_bytes()),
			dependencies: BTreeSet::new()
		})
	}

	fn markdown_version(&self) -> u8 {
		match self {
			Self::V1(_, info) => info.markdown_version
		}
	}

	fn is_template_up2date(&self, template: &str) -> bool {
		let hash = blake3::hash(template.as_bytes());
		match self {
			Self::V1(_, info) => info.template_hash == hash
		}
	}

	fn is_rustdoc_up2date(&self, rustdoc: &str) -> bool {
		let hash = blake3::hash(rustdoc.as_bytes());
		match self {
			Self::V1(_, info) => info.rustdoc_hash == hash
		}
	}

	fn is_empty(&self) -> bool {
		match self {
			Self::V1(_, info) => info.dependencies.is_empty()
		}
	}

	fn dependencies(&self) -> BTreeMap<&str, (Option<&Version>, &str)> {
		match self {
			Self::V1(_, info) => info
				.dependencies
				.iter()
				.map(|dep| (dep.crate_name(), (dep.version(), dep.lib_name())))
				.collect()
		}
	}

	fn add_dependency(&mut self, crate_name: String, version: Option<Version>, lib_name: String) {
		match self {
			Self::V1(_, info) => {
				info.dependencies
					.insert(Dependency::new(crate_name, version, lib_name));
			}
		}
	}
}

pub struct DependencyInfo(DependencyInfoImpl);

impl DependencyInfo {
	/// Return the current markdown version. This is just an internal number to track changes
	/// to the markdown output, and does not correspond to any "official" markdown spec version.
	#[inline]
	pub fn markdown_version() -> u8 {
		0
	}

	pub fn new(template: &str, rustdoc: &str) -> Self {
		Self(DependencyInfoImpl::new(
			Self::markdown_version(),
			template,
			rustdoc
		))
	}

	pub fn decode(data: String) -> anyhow::Result<Self> {
		let bytes = base64::decode_config(data, URL_SAFE_NO_PAD)?;
		Ok(Self(serde_cbor::from_slice(&bytes)?))
	}

	pub fn encode(&self) -> String {
		base64::encode_config(&serde_cbor::to_vec(&self.0).unwrap(), URL_SAFE_NO_PAD)
	}

	pub fn check_input(&self, template: &str, rustdoc: &str) -> bool {
		self.0.is_template_up2date(template) && self.0.is_rustdoc_up2date(rustdoc)
	}

	pub fn is_empty(&self) -> bool {
		self.0.is_empty()
	}

	pub fn add_dependency(
		&mut self,
		crate_name: String,
		version: Option<Version>,
		lib_name: String
	) {
		self.0.add_dependency(crate_name, version, lib_name)
	}

	pub fn check_outdated(&self) -> bool {
		self.0.markdown_version() != Self::markdown_version()
	}

	pub fn check_dependency(
		&self,
		crate_name: &str,
		version: Option<&Version>,
		lib_name: &str,
		allow_missing: bool
	) -> bool {
		// check that dependency is present
		let dependencies = self.0.dependencies();
		let (dep_ver, dep_lib_name) = match dependencies.get(crate_name) {
			Some(dep) => dep,
			None => return allow_missing
		};

		// check that the lib names match
		if lib_name != *dep_lib_name {
			return false;
		}

		// check that the versions are compatible
		// if the requested version is None, we accept all versions
		// otherwise, we expect a concrete version that is semver-compatible
		if let Some(ver) = version {
			match dep_ver {
				None => return false,
				Some(dep_ver) if *dep_ver < ver => return false,
				_ => {}
			}
		}

		true
	}
}

#[cfg(test)]
mod tests {
	use super::DependencyInfo;
	use base64::URL_SAFE_NO_PAD;
	use semver::Version;

	const TEMPLATE: &str = include_str!("README.j2");
	const RUSTDOC: &str = "This is the best crate ever!";

	#[test]
	fn test_dep_info() {
		let mut dep_info = DependencyInfo::new(TEMPLATE, RUSTDOC);

		assert!(dep_info.check_input(TEMPLATE, RUSTDOC));
		assert!(!dep_info.check_input(TEMPLATE, ""));
		assert!(!dep_info.check_input("", RUSTDOC));

		// check that it is initially empty
		assert!(dep_info.is_empty());
		assert!(!dep_info.check_dependency("anyhow", None, "anyhow", false));
		assert!(dep_info.check_dependency("anyhow", None, "anyhow", true));

		let version_1_0_0: Version = "1.0.0".parse().unwrap();
		let version_1_0_1: Version = "1.0.1".parse().unwrap();
		let version_1_1_0: Version = "1.1.0".parse().unwrap();

		dep_info.add_dependency(
			"anyhow".into(),
			Some(version_1_0_1.clone()),
			"anyhow".into()
		);
		assert!(dep_info.check_dependency("anyhow", None, "anyhow", false));
		assert!(dep_info.check_dependency("anyhow", Some(&version_1_0_0), "anyhow", false));
		assert!(dep_info.check_dependency("anyhow", Some(&version_1_0_1), "anyhow", false));
		assert!(!dep_info.check_dependency("anyhow", Some(&version_1_1_0), "anyhow", false));
		assert!(!dep_info.check_dependency("anyhow", Some(&version_1_0_0), "any_how", false));

		// check that encoding and decoding works as expected
		let encoded = dep_info.encode();
		println!(
			"encoded: {}",
			hex::encode_upper(base64::decode_config(&encoded, URL_SAFE_NO_PAD).unwrap())
		);
		let dep_info = DependencyInfo::decode(encoded).unwrap();
		assert!(dep_info.check_input(TEMPLATE, RUSTDOC));
		assert!(dep_info.check_dependency("anyhow", Some(&version_1_0_1), "anyhow", false));
	}
}
