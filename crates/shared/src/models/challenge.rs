use nalgebra::DMatrix;
use serde::{
    de::{self, Visitor},
    Deserialize, Deserializer, Serialize, Serializer,
};
use std::fmt;

#[derive(Debug, Clone)]
pub struct FixedF64(pub f64);

impl Serialize for FixedF64 {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        // adjust precision as needed
        serializer.serialize_str(&format!("{:.12}", self.0))
    }
}

impl<'de> Deserialize<'de> for FixedF64 {
    fn deserialize<D>(deserializer: D) -> Result<FixedF64, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct FixedF64Visitor;

        impl Visitor<'_> for FixedF64Visitor {
            type Value = FixedF64;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a string representing a fixed precision float")
            }

            fn visit_str<E>(self, value: &str) -> Result<FixedF64, E>
            where
                E: de::Error,
            {
                value
                    .parse::<f64>()
                    .map(FixedF64)
                    .map_err(|_| E::custom(format!("invalid f64: {value}")))
            }
        }

        deserializer.deserialize_str(FixedF64Visitor)
    }
}

impl PartialEq for FixedF64 {
    fn eq(&self, other: &Self) -> bool {
        format!("{:.10}", self.0) == format!("{:.10}", other.0)
    }
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq)]
pub struct ChallengeRequest {
    pub rows_a: usize,
    pub cols_a: usize,
    pub data_a: Vec<FixedF64>,
    pub rows_b: usize,
    pub cols_b: usize,
    pub data_b: Vec<FixedF64>,
    pub timestamp: Option<u64>,
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq)]
pub struct ChallengeResponse {
    pub result: Vec<FixedF64>,
    pub rows: usize,
    pub cols: usize,
}

pub fn calc_matrix(req: &ChallengeRequest) -> ChallengeResponse {
    // convert FixedF64 to f64
    let data_a: Vec<f64> = req.data_a.iter().map(|x| x.0).collect();
    let data_b: Vec<f64> = req.data_b.iter().map(|x| x.0).collect();
    let a = DMatrix::from_vec(req.rows_a, req.cols_a, data_a);
    let b = DMatrix::from_vec(req.rows_b, req.cols_b, data_b);
    let c = a * b;

    let data_c: Vec<FixedF64> = c.iter().map(|x| FixedF64(*x)).collect();

    ChallengeResponse {
        rows: c.nrows(),
        cols: c.ncols(),
        result: data_c,
    }
}
