use super::*;

#[test]
fn every_record_reads_back_and_a_malformed_one_is_refused() {
    let records = [
        Record::Verdict(Verdict {
            passed: 75,
            failed: 2,
            expected: 77,
        }),
        Record::Protections(Protections {
            write_protect: true,
            smep: false,
            smap: true,
            umip: false,
            nx: true,
        }),
    ]
    .into_iter()
    .chain(Fact::ALL.map(Record::Fact));
    for record in records {
        let line = format!("{LEVEL}{record}");
        assert_eq!(Record::parse(&line), Some(Ok(record)), "{line}");
    }

    for (body, error) in [
        ("v2 fact name=settled", Error::UnknownVersion),
        ("v01 fact name=settled", Error::Malformed),
        ("1 fact name=settled", Error::Malformed),
        ("v1 omen name=settled", Error::UnknownKind),
        ("v1 fact name=unheard-of", Error::UnknownFact),
        ("v1 fact", Error::Malformed),
        ("v1 fact name=", Error::Malformed),
        ("v1 fact title=settled", Error::Malformed),
        ("v1 fact name=settled ", Error::Malformed),
        ("v1  fact name=settled", Error::Malformed),
        ("v1 verdict passed=76 failed=0", Error::Malformed),
        (
            "v1 verdict failed=0 passed=76 expected=76",
            Error::Malformed,
        ),
        (
            "v1 verdict passed=+76 failed=0 expected=76",
            Error::Malformed,
        ),
        (
            "v1 verdict passed=076 failed=0 expected=76",
            Error::Malformed,
        ),
        (
            "v1 verdict passed=76 failed=0 expected=99999999999999999999999",
            Error::Malformed,
        ),
        (
            "v1 protections wp=1 smep=2 smap=1 umip=1 nx=1",
            Error::Malformed,
        ),
    ] {
        let line = format!("{LEVEL}{OPEN}{body}");
        assert_eq!(Record::parse(&line), Some(Err(error)), "{line}");
    }

    for line in [
        "[warn ] record v1 fact name=settled",
        "[info ] recorded v1",
        "record v1",
    ] {
        assert_eq!(Record::parse(line), None, "{line}");
    }
}
