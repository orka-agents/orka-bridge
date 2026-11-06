// DIAGNOSTIC-ONLY derivation shared by build.rs and portable tests. The sealed
// source is never modified. The derived accessor keeps raw PID/error evidence;
// it neither changes nor replaces the production all-or-nothing stop proof.
pub const MARKER: &str = "\n    impl OwnedQemuProcess {\n";
pub const METHOD_NAME: &str = "diagnostic_member_handles";
pub const TYPES: &str = r#"
    pub type DiagnosticMember = (u32, io::Result<Handle>);
    pub struct DiagnosticJobMembers {
        pub total: u32,
        pub active: u32,
        pub assigned: u32,
        pub complete: bool,
        pub entries: Vec<DiagnosticMember>,
    }
"#;
pub const METHOD: &str = r#"        /// DIAGNOSTIC ONLY; absent from product source and qualification builds.
        pub fn diagnostic_member_handles(
            &self,
            deadline: std::time::Instant,
        ) -> io::Result<DiagnosticJobMembers> {
            let job = self.resources().job.0;
            let info = accounting(job)?;
            let capacity = 256usize;
            let offset = std::mem::offset_of!(JOBOBJECT_BASIC_PROCESS_ID_LIST, ProcessIdList);
            let bytes = offset + capacity * std::mem::size_of::<usize>();
            let mut storage = vec![0usize; bytes.div_ceil(std::mem::size_of::<usize>())];
            let ok = unsafe { QueryInformationJobObject(job, JobObjectBasicProcessIdList,
                storage.as_mut_ptr().cast(), bytes as u32, std::ptr::null_mut()) };
            let error = (ok == 0).then(io::Error::last_os_error);
            if let Some(error) = &error {
                if error.raw_os_error() != Some(ERROR_MORE_DATA as i32) {
                    return Err(io::Error::new(error.kind(), format!("Job PID query: {error}")));
                }
            }
            let header = unsafe { &*storage.as_ptr().cast::<JOBOBJECT_BASIC_PROCESS_ID_LIST>() };
            let listed = header.NumberOfProcessIdsInList as usize;
            if listed > capacity { return Err(io::ErrorKind::InvalidData.into()); }
            let complete = ok != 0 && listed == header.NumberOfAssignedProcesses as usize;
            let ids = unsafe { std::slice::from_raw_parts(
                storage.as_ptr().cast::<u8>().add(offset).cast::<usize>(), listed) };
            let mut observations = Vec::with_capacity(ids.len());
            for id in ids {
                let id = u32::try_from(*id).map_err(|_| io::ErrorKind::InvalidData)?;
                let member = (|| {
                    if std::time::Instant::now() >= deadline {
                        return Err(io::ErrorKind::TimedOut.into());
                    }
                    let process = unsafe { Handle::new(OpenProcess(
                        0x00100000 | PROCESS_QUERY_LIMITED_INFORMATION, 0, id))? };
                    let mut member = 0;
                    unsafe { check(IsProcessInJob(process.0, job, &mut member))?; }
                    if member == 0 { return Err(io::Error::new(io::ErrorKind::InvalidData,
                        "PID is no longer a member of this owned Job")); }
                    Ok(process)
                })();
                // A failed PID is evidence, never a reason to discard other PIDs.
                observations.push((id, member));
            }
            Ok(DiagnosticJobMembers { total: info.TotalProcesses, active: info.ActiveProcesses,
                assigned: header.NumberOfAssignedProcesses, complete, entries: observations })
        }
"#;
pub fn derive(source: &str) -> Result<String, String> {
    if source.contains(METHOD_NAME) {
        return Err(format!("source already mentions {METHOD_NAME}"));
    }
    let mut found = source.match_indices(MARKER);
    let (at, _) = found
        .next()
        .ok_or_else(|| "OwnedQemuProcess impl marker missing".to_owned())?;
    if found.next().is_some() {
        return Err("OwnedQemuProcess impl marker is not unique".to_owned());
    }
    let split = at + MARKER.len();
    Ok(format!(
        "{}{TYPES}{}{METHOD}{}",
        &source[..at],
        &source[at..split],
        &source[split..]
    ))
}
