use crate::workdir::Workdir;

#[test]
fn validate_good_csv() {
    let wrk = Workdir::new("validate").flexible(true);
    wrk.create(
        "data.csv",
        vec![
            svec!["title", "name", "age"],
            svec!["Professor", "Xaviers", "60"],
            svec!["Prisoner", "Magneto", "90"],
            svec!["First Class Student", "Iceman", "14"],
        ],
    );
    let mut cmd = wrk.command("validate");
    cmd.arg("data.csv");

    wrk.output(&mut cmd);
}

#[test]
fn validate_bad_csv() {
    let wrk = Workdir::new("validate").flexible(true);
    wrk.create(
        "data.csv",
        vec![
            svec!["title", "name", "age"],
            svec!["Professor", "Xaviers", "60"],
            svec!["Magneto", "90",],
            svec!["First Class Student", "Iceman", "14"],
        ],
    );
    let mut cmd = wrk.command("validate");
    cmd.arg("data.csv");

    wrk.assert_err(&mut cmd);
}

#[test]
fn validate_adur_public_toilets_dataset_with_json_schema() {

    let wrk = Workdir::new("validate").flexible(true);

    // copy schema file to workdir
    let schema: String = wrk.load_test_resource("public-toilets-schema.json");
    wrk.create_from_string("schema.json", &schema);
 
    // copy csv file to workdir
    let csv: String = wrk.load_test_resource("adur-public-toilets.csv");
    wrk.create_from_string("data.csv", &csv);

    // run validate command
    let mut cmd = wrk.command("validate");
    cmd.arg("data.csv").arg("schema.json");

    wrk.output(&mut cmd);

    // check invalid file output
    
    // invalid records with index from original csv
    // row 1: missing values for ExtractDate and OrganisationLabel
    // row 3: wrong value for CoordinateReferenceSystem and Category
    // note: removed unnecessary quotes for string column "OpeningHours"
    let invalid_expected = r#"ExtractDate,OrganisationURI,OrganisationLabel,ServiceTypeURI,ServiceTypeLabel,LocationText,CoordinateReferenceSystem,GeoX,GeoY,GeoPointLicensingURL,Category,AccessibleCategory,RADARKeyNeeded,BabyChange,FamilyToilet,ChangingPlace,AutomaticPublicConvenience,FullTimeStaffing,PartOfCommunityScheme,CommunitySchemeName,ChargeAmount,InfoURL,OpeningHours,ManagedBy,ReportEmail,ReportTel,Notes,UPRN,Postcode,StreetAddress,GeoAreaURI,GeoAreaLabel
  ,http://opendatacommunities.org/id/district-council/adur,,http://id.esd.org.uk/service/579,Public toilets,BEACH GREEN PUBLIC CONVENIENCES BRIGHTON ROAD LANCING,OSGB36,518072,103649,http://www.ordnancesurvey.co.uk/business-and-government/help-and-support/public-sector/guidance/derived-data-exemptions.html,Female and male,Unisex,Yes,No,No,No,No,No,No,,,http://www.adur-worthing.gov.uk/streets-and-travel/public-toilets/,S = 09:00 - 21:00 W = 09:00 - 17:00 ,ADC,surveyors@adur-worthing.gov.uk,01903 221471,,60001449,,BEACH GREEN PUBLIC CONVENIENCES BRIGHTON ROAD LANCING,,
2014-07-07 00:00,http://opendatacommunities.org/id/district-council/adur,Adur,http://id.esd.org.uk/service/579,Public toilets,PUBLIC CONVENIENCES SHOPSDAM ROAD LANCING,OSGB3,518915,103795,http://www.ordnancesurvey.co.uk/business-and-government/help-and-support/public-sector/guidance/derived-data-exemptions.html,Mens,Unisex,Yes,No,No,No,No,No,No,,,http://www.adur-worthing.gov.uk/streets-and-travel/public-toilets/,S = 09:00 - 21:00 W = 09:00 - 17:00,ADC,surveyors@adur-worthing.gov.uk,01903 221471,,60007428,,,,
"#;
    let invalid_output: String = wrk.from_str(&wrk.path("data.csv.invalid"));
    assert_eq!(invalid_expected.to_string(), invalid_output);

    // check validation error output

    let validation_errors_expected = r#"{"valid":false,"errors":[{"keywordLocation":"/properties/ExtractDate/type","instanceLocation":"/ExtractDate","absoluteKeywordLocation":"https://example.com/properties/ExtractDate/type","error":"null is not of type \"string\""},{"keywordLocation":"/properties/OrganisationLabel/type","instanceLocation":"/OrganisationLabel","absoluteKeywordLocation":"https://example.com/properties/OrganisationLabel/type","error":"null is not of type \"string\""}],"row_index":1}
{"valid":false,"errors":[{"keywordLocation":"/properties/CoordinateReferenceSystem/pattern","instanceLocation":"/CoordinateReferenceSystem","absoluteKeywordLocation":"https://example.com/properties/CoordinateReferenceSystem/pattern","error":"\"OSGB3\" does not match \"(WGS84|OSGB36)\""},{"keywordLocation":"/properties/Category/pattern","instanceLocation":"/Category","absoluteKeywordLocation":"https://example.com/properties/Category/pattern","error":"\"Mens\" does not match \"(Female|Male|Female and Male|Unisex|Male urinal|Children only|None)\""}],"row_index":3}
"#;
    let validation_error_output: String =
        wrk.from_str(&wrk.path("data.csv.validation-errors.jsonl"));
    assert_eq!(
        validation_errors_expected.to_string(),
        validation_error_output
    );
}
