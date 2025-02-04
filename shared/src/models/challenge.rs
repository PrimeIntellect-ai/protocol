use nalgebra::DMatrix;
use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct ChallengeRequest {
    pub rows_a: usize,
    pub cols_a: usize,
    pub data_a: Vec<f64>,
    pub rows_b: usize,
    pub cols_b: usize,
    pub data_b: Vec<f64>,
}

#[derive(Deserialize, Serialize, PartialEq, Debug)]
pub struct ChallengeResponse {
    pub result: Vec<f64>,
    pub rows: usize,
    pub cols: usize,
}

pub fn calc_matrix(req: &ChallengeRequest) -> ChallengeResponse {
    let a = DMatrix::from_vec(req.rows_a, req.cols_a, req.data_a.clone());
    let b = DMatrix::from_vec(req.rows_b, req.cols_b, req.data_b.clone());
    let c = a * b;

    ChallengeResponse {
        rows: c.nrows(),
        cols: c.ncols(),
        result: c.data.as_vec().clone(),
    }
}
