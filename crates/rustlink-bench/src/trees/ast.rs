//! `ast`: a random expression tree built bottom-up, then evaluated.

use super::run_variant;
use crate::measure::{Params, Pass};
use crate::plan::Plan;
use crate::variants::{TreeBuild, VARIANTS, eval, with_variant};
use crate::workload::{Outcome, Workload};

/// Operands are created before their operation, so most links point a short way back.
pub struct Ast;

impl Workload for Ast {
    fn name(&self) -> &'static str {
        "ast"
    }

    fn variants(&self) -> &'static [&'static str] {
        VARIANTS
    }

    fn run(&self, variant: &str, params: &Params) -> Result<Outcome, String> {
        let plan = Plan::expression(params.n.max(1), params.seed);
        let reach = plan.reach();
        let nodes = plan.nodes.len();

        with_variant!(variant, V => run_variant::<V>(
            params,
            nodes,
            &reach,
            "visit",
            || V::from_plan(&plan),
            |tree| {
                let (value, visits) = eval(tree);
                Pass { checksum: u64::from(value), units: visits }
            },
        ))
    }
}
