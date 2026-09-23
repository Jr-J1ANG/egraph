use egg::{Rewrite, rewrite};

use crate::lang::FheLang;

pub fn rules() -> Vec<Rewrite<FheLang, ()>> {
    vec![
        // Factorization / expansion
        rewrite!(
            "factor-left—1";
            "(+ (* ?a ?b) (* ?a ?c))" => "(* ?a (+ ?b ?c))"
        ),
        rewrite!(
            "factor-left—2";
            "(+ (* ?a ?b) (* ?c ?a))" => "(* ?a (+ ?b ?c))"
        ),
        rewrite!(
            "factor-left—3";
            "(+ (* ?b ?a) (* ?a ?c))" => "(* ?a (+ ?b ?c))"
        ),
        rewrite!(
            "factor-left—4 ";
            "(+ (* ?b ?a) (* ?c ?a))" => "(* ?a (+ ?b ?c))"
        ),
        rewrite!(
            "expand-left";
            "(* ?a (+ ?b ?c))" => "(+ (* ?a ?b) (* ?a ?c))"
        ),
        // Multiplication reassociation
        rewrite!(
            "assoc-mul-l1";
            "(* (* ?a ?b) ?c)" => "(* (* ?b ?c) ?a)"
        ),
        rewrite!(
            "assoc-mul-l2";
            "(* (* ?a ?b) ?c)" => "(* (* ?a ?c) ?b )"
        ),
        rewrite!(
            "assoc-mul-r1";
            "(* ?a (* ?b ?c))" => "(* ?b (* ?a ?c))"
        ),
        rewrite!(
            "assoc-mul-r2";
            "(* ?a (* ?b ?c))" => "(* ?c (* ?a ?b))"
        ),
        
        
        // Addition reassociation
        rewrite!(
            "assoc-add-l1";
            "(+ (+ ?a ?b) ?c)" => "(+ (+ ?b ?c) ?a)"
        ),
        rewrite!(
            "assoc-add-l2";
            "(+ (+ ?a ?b) ?c)" => "(+ (+ ?a ?c) ?b )"
        ),
        rewrite!(
            "assoc-add-r1";
            "(+ ?a (+ ?b ?c))" => "(+ ?b (+ ?a ?c))"
        ),
        rewrite!(
            "assoc-add-r2";
            "(+ ?a (+ ?b ?c))" => "(+ ?c (+ ?a ?b))"
        ),
        
    ]
}

