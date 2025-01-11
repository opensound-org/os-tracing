#![cfg_attr(nightly, feature(doc_auto_cfg))]

pub mod async_req_res;
pub mod stop;
pub mod tmp;
pub mod tracing_msg;

pub fn add(left: usize, right: usize) -> usize {
    left + right
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_works() {
        let result = add(2, 2);
        assert_eq!(result, 4);
    }
}
